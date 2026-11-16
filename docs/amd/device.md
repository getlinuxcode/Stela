# Le périphérique Metal de Stela sur la RX 9070 XT, sous Linux (J4)

Le 16 novembre 2026, le vrai périphérique Metal de Stela a tourné pour la première fois sur la Radeon. C'est le même code que celui qui servira macOS : `Device` sur `NativeBackend` et `AirCompiler`. Il tournait ici sous Linux, sur RADV.

```text
Metal device on 0000:07:00.0:
  Metal device: open on RADV, with the AIR compiler for GFX12
  buffer sum: 65536 floats added on the GPU and checked (628.81µs)
  triangle: drawn into a 256x256 texture and checked, 18432 pixels covered (964.96µs)
  line strip: the same unspecified-class pipeline drew 383 pixels (336.62µs)
RESULT: Stela's Metal device ran its work on the GPU.
```

- **Calcul** : un noyau Metal additionne deux buffers de 65 536 flottants, et le CPU vérifie chaque valeur.
- **Rendu** : un triangle aux sommets rouge, vert et bleu est dessiné dans une texture RGBA8, puis recopié dans un buffer partagé. Il couvre exactement l'aire attendue (18 432 pixels), et chaque coin a la couleur de son sommet.
- **Topologie** : le même pipeline, sans classe de topologie, dessine une bande de lignes. Sur RADV, qui ne fournit pas `dynamicPrimitiveTopologyUnrestricted`, ce dessin passe par la variante « lignes » du correctif sur la topologie (J2). Ce correctif est donc vérifié sur le matériel.

Les shaders sont dans `crates/stela-amd-host/metal/first.metal`, compilés en metallib sur un Mac.

## Comment ça s'assemble

Stela ne passe pas ses shaders au pilote en SPIR-V. Il lui remet directement son NIR, par la structure interne de Mesa `VkPipelineShaderStageNirCreateInfoMESA`. Le compilateur et le pilote doivent donc être le même Mesa, dans la même bibliothèque : le NIR pointe vers les tables de types GLSL de la bibliothèque qui l'a construit. Sur NVIDIA, NVK et le compilateur sont liés ensemble dans `stela.efi`. Sous Linux, l'équivalent AMD est `libstela_radv.so` :

- RADV, construit à partir du Mesa de Stela (26.2.5, avec sa série de correctifs) ;
- le compilateur Metal de Stela (`stela_metal_*`) ;
- `stela_radv_nir_options`, l'équivalent de `stela_nvk_nir_options` (`crates/stela-driver-gpu/src/amd/radv_host/`).

Il a fallu un correctif dans RADV, `third_party/patches/mesa/9001-radv-take-nir-stages.patch`. RADV initialise ses étages de pipeline lui-même et ignorait un étage sans module qui porte cette structure NIR. NVK, lui, passe par le chemin commun de Mesa, qui la lit. Le correctif est numéroté `9001` pour passer après la série d'upstream. `cargo xtask gpu export` le renumérote : il faut alors lui redonner son nom à la main.

La mémoire partagée Metal est celle de l'invité, exportée en dma-buf. Ici, l'invité est le processus lui-même : chaque buffer partagé est un memfd transformé en dma-buf par `/dev/udmabuf`, ce qui demande le groupe `kvm`.

## Refaire l'essai

Prérequis Ubuntu 24.04 : `glslang-tools libdrm-dev libudev-dev libelf-dev bison flex`, LLVM 23, et l'utilisateur dans le groupe `kvm`.

```sh
cargo xtask gpu fetch
# libdrm 2.4.133, que RADV demande (Ubuntu fournit la 2.4.125), depuis le commit épinglé
C=$(git ls-tree HEAD third_party/libdrm | awk '{print $3}')
mkdir -p target/gpu/libdrm-src && git --git-dir=.git/modules/libdrm archive $C | tar -x -C target/gpu/libdrm-src
export PATH=$PWD/.tools/gpu/generators/bin:$PWD/.tools/gpu/rust/bin:$PATH
meson setup target/gpu/libdrm-build target/gpu/libdrm-src --prefix=$PWD/target/gpu/libdrm-install \
  --libdir=lib -Dbuildtype=release -Damdgpu=enabled -Dradeon=disabled -Dnouveau=disabled \
  -Dintel=disabled -Dvmwgfx=disabled -Dtests=false -Dman-pages=disabled -Dcairo-tests=disabled \
  -Dvalgrind=disabled
ninja -C target/gpu/libdrm-build install
# libstela_radv.so
export CLANG_PATH=/usr/lib/llvm-23/bin/clang PKG_CONFIG=/usr/bin/pkg-config \
  PKG_CONFIG_PATH=$PWD/target/gpu/libdrm-install/lib/pkgconfig LLVM_CONFIG=/usr/bin/llvm-config-23
meson setup target/gpu/radv-host crates/stela-driver-gpu/src -Dcompilers=amd -Dradv-host=true \
  -Dair-llvm-library=/usr/lib/llvm-23/lib/libLLVM-23.so -Db_ndebug=true
ninja -C target/gpu/radv-host amd/radv_host/libstela_radv.so
# Le metallib, sur un Mac
xcrun -sdk macosx metal -o first.metallib crates/stela-amd-host/metal/first.metal
# L'essai
cargo build -p stela-amd-host --release --features radv-host
sg kvm -c "./target/release/stela-amd-host --device first.metallib"
```

`-Db_ndebug=true` coupe une assertion de la couche Vulkan commune de Mesa. Elle exige que le NIR reçu porte exactement les options NIR de l'étage selon RADV, alors que le compilateur Metal prend un seul jeu d'options par périphérique : celui de l'étage de calcul. Les deux jeux ne diffèrent que par une préférence sur les multiplications-additions des étages de géométrie.

Pour vérifier que ce RADV admet toujours la carte, sans `--device` :

```sh
VK_DRIVER_FILES=<un fichier ICD JSON pointant sur libstela_radv.so> ./target/release/stela-amd-host
```

## La suite Metal sur la carte

`stela-amd-host --suite DIR` exécute les cas de la suite Metal sur ce périphérique, comme l'exécuteur du Mac (`runner.m`). Il prend les bibliothèques que le Mac a compilées (`DIR/plan/lib`) et écrit ses résultats là où `cargo xtask gpu metal-suite check` les lit (`DIR/results/linux-gfx1201`). Chaque cas commence par l'écriture d'un statut « plantage », remplacé par le vrai résultat à la fin : après un abandon du pilote, une nouvelle exécution reprend au cas suivant.

L'exécuteur prépare les ressources d'un cas, puis encode ses étapes. Un cas simple est une séquence d'une seule étape. Il reproduit :

- les calculs, avec textures, échantillonneurs, constantes de fonction, entrée d'étage et dispatch par fils ou par groupes ;
- les rendus, avec attachements de couleur, de profondeur et de stencil, multi-échantillonnage et résolution, descripteurs de sommets, tracés indexés ou multiples, cibles en tableau, et l'état de l'encodeur (stencil, biais, élimination, viewport, scissor) ;
- les blits ;
- les séquences, qui enchaînent ces étapes avec de nouveaux command buffers et des événements ;
- la tessellation et le mesh (pipelines, tracés directs et indirects) ;
- les heaps (automatiques ou à placement, partagés ou privés), les argument buffers de Tier 2 (adresses de buffers, identifiants de textures, d'échantillonneurs, d'indirect command buffers et de pipelines) et les indirect command buffers (commandes encodées par le CPU ou par le GPU, copiées, réinitialisées) ;
- les vues de textures, les liaisons à un décalage, `inline` ou avec des bornes de LOD, plusieurs viewports, les résultats de visibilité et les tracés indirects.

Les textures de tous les types que crée la suite (1D, 2D, 3D, cube, tableaux, multi-échantillonnées) sont chargées et relues niveau par niveau, et par aspect pour la profondeur et le stencil (`suite_resources.rs`). Les familles `host` (69 cas) et `framework` (56 cas) testent les API de macOS et les frameworks d'Apple : elles ne peuvent tourner qu'avec un macOS invité, en J7.

```sh
cargo xtask gpu metal-suite mac --host USER@MAC       # plan, bibliothèques et résultats de référence
python3 -I xtask/scripts/amd-runtime-libraries.py --host USER@MAC   # bibliothèques d'exécution
sg kvm -c "./target/release/stela-amd-host --suite .stela/metal-suite"
cargo xtask gpu metal-suite check .stela/metal-suite/results/linux-gfx1201
```

Mesure du 16 novembre 2026, sur une exécution complète et neuve de la suite :

| | Cas |
| --- | --- |
| Identiques au Mac, dans ses tolérances | 13 829 (97,6 % de la suite) |
| Écarts | 15, voir plus bas |
| Non pris en charge | 13 : 3 bibliothèques d'exécution sans équivalent en ligne de commande (arrondi des conversions, threads requis) et 10 cas qui demandent une famille (Apple 9, Apple 10) ou une capacité que le périphérique n'annonce pas |
| Pas encore exécutés | 266, dont 125 qui demandent macOS (`host`, `framework`) |

Corrections faites en route, que RADV avec Stela demandait :

- **Patch RADV `9001`.** Le NIR que Stela remet à RADV ne passe pas par les transformations de fin du chemin SPIR-V de RADV, et ACO refusait ce qui en dépend. Le patch applique désormais à ce NIR ce qu'il lui faut : coordonnée de point et identifiant de primitive en entrées, `load_base_vertex` pris comme le `BaseVertex` de Vulkan, lectures globales bornées protégées par un test, doubles abaissés, sinus et cosinus normalisés.
- **Largeur SIMD.** Stela prenait `maxSubgroupSize` comme `threadExecutionWidth`, soit 64 sur RADV. Le code Metal écrit pour les GPU Apple compte sur des simdgroups de 32 : Stela prend maintenant 32 dès que le pilote le permet (`capabilities::simd_width`).
- **Position du fragment.** Le frontal AIR construit maintenant la position avec `nir_build_frag_coord`, qui suit la forme demandée par le pilote (RADV : `xy`, `z` et `1/w` séparés).
- **Résolution dans un plan 3D (patch RADV `9002`).** RADV résolvait la couleur d'un rendu dans le plan 0 d'une texture 3D, quel que soit le plan visé. Une telle résolution passe maintenant par le chemin de calcul, avec le plan en décalage z.
- **Accès non uniformes (patch RADV `9003`).** Metal laisse chaque fil d'une wave lire sa propre texture à travers un argument buffer. RADV ne protège par une boucle que les accès marqués non uniformes : toute la wave lisait la texture d'un seul fil. Chaque accès de texture ou d'image du NIR de Stela est maintenant marqué, puis `nir_opt_non_uniform_access` retire la marque là où l'identifiant est uniforme.
- **Un scissor par viewport.** Metal ne découpe aucun viewport tant que l'encodeur ne pose pas de scissor, alors que Vulkan exige autant de scissors que de viewports. Plusieurs viewports posés sans scissor étaient tracés avec un seul. Chacun a maintenant un scissor qui couvre la cible (`command_buffer/render.rs`).
- **Commandes générées par le GPU sur RADV.** Sans ensemble d'exécution, `VK_EXT_device_generated_commands` demande le pipeline lié (`VkGeneratedCommandsPipelineInfoEXT`) : RADV mesure les séquences avec et plantait sans lui. Le pipeline est maintenant transmis. RADV prétraite aussi dans ses types de mémoire à adresses 32 bits, et Stela refusait tant que tous les types de mémoire d'upload ne convenaient pas : un seul type convenable suffit, puisque les blocs d'upload sont des buffers d'usage « preprocess », que le pilote garde dans ces types.

Les 13 écarts :

| Écart | Cas | Cause |
| --- | --- | --- |
| Filtrage anisotrope | 5 | Sa qualité est propre à chaque matériel : Metal ne la fixe pas. |
| PVRTC en stockage partagé | 6 | Le Mac charge et lit ces textures par le CPU. L'exécuteur passe par un blit, faute de mémoire d'invité sous Linux. C'est une limite de l'exécuteur, à revoir avec macOS invité. |
| Cibles `rgb10a2unorm` et `bgr10a2unorm` | 2 | Une unité de moins sur certains pixels. Pour ces formats, RADV fait sortir la couleur en flottant 16 bits (`ac_choose_spi_color_formats`), ce que son code présente comme requis avec RB+. |
| Tracés encodés par le GPU qui changent de pipeline ou de pas de sommets | 2 | `sequence.icb.gpu_indexed_draw` et `sequence.icb.gpu_draw_dynamic_stride` ne tracent rien. Les mêmes tracés encodés par le CPU, et les tracés GPU qui héritent du pipeline, passent. Le défaut est dans les commandes générées de rendu de RADV, avec un ensemble d'exécution ou un pas de sommets dynamique : à examiner. |

Les bibliothèques `*_runtime_*`, que le Mac compile à partir du MSL pendant le cas, sont compilées à l'avance sur le Mac par `xtask/scripts/amd-runtime-libraries.py`, avec les options de ligne de commande équivalentes (langage, fast math, mode mathématique, macros…), puis chargées depuis `plan/lib-runtime`. Les 7 bibliothèques dont une option n'a pas d'équivalent en ligne de commande restent de côté.

## Ce qui reste pour J4

- Étendre l'exécuteur aux familles qui restent sous Linux (141 cas) : tables de fonctions visibles (29), tuiles et imageblocks (23), réflexion (10), modes de stockage (8), bibliothèques dynamiques (7), fonctions liées (7), argument encoders (8), événements et files (9), compteurs, journal, et quelques cas isolés.
- Les deux tracés encodés par le GPU qui ne tracent rien.
- Mesurer les alignements réels (lignes et décalages des surfaces linéaires) sur des images linéaires de RADV, pour remplacer les 256 octets estimés de `metal/amd.rs`.
