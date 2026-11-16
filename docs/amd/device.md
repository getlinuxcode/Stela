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
- les séquences, qui enchaînent ces étapes avec de nouveaux command buffers et des événements.

Les textures de tous les types que crée la suite (1D, 2D, 3D, cube, tableaux, multi-échantillonnées) sont chargées et relues niveau par niveau, et par aspect pour la profondeur et le stencil (`suite_resources.rs`). Les familles `host` (69 cas) et `framework` (56 cas) testent les API de macOS et les frameworks d'Apple : elles ne peuvent tourner qu'avec un macOS invité, en J7.

```sh
cargo xtask gpu metal-suite mac --host USER@MAC       # plan, bibliothèques et résultats de référence
sg kvm -c "./target/release/stela-amd-host --suite .stela/metal-suite"
cargo xtask gpu metal-suite check .stela/metal-suite/results/linux-gfx1201
```

Mesure du 16 novembre 2026 :

| | Cas |
| --- | --- |
| Identiques au Mac, dans ses tolérances | 12 848 |
| Écarts | 13, voir plus bas |
| Non pris en charge | 740 : 737 bibliothèques `*_runtime_*`, que le Mac compile à partir du MSL pendant le cas, et 3 cas qui demandent une capacité que le périphérique n'annonce pas |
| Pas encore exécutés | 525, dont 125 qui demandent macOS (`host`, `framework`) |

Corrections faites en route, que RADV avec Stela demandait :

- **Patch RADV `9001`.** Le NIR que Stela remet à RADV ne passe pas par les transformations de fin du chemin SPIR-V de RADV, et ACO refusait ce qui en dépend. Le patch applique désormais à ce NIR ce qu'il lui faut : coordonnée de point et identifiant de primitive en entrées, `load_base_vertex` pris comme le `BaseVertex` de Vulkan, lectures globales bornées protégées par un test, doubles abaissés, sinus et cosinus normalisés.
- **Largeur SIMD.** Stela prenait `maxSubgroupSize` comme `threadExecutionWidth`, soit 64 sur RADV. Le code Metal écrit pour les GPU Apple compte sur des simdgroups de 32 : Stela prend maintenant 32 dès que le pilote le permet (`capabilities::simd_width`).
- **Position du fragment.** Le frontal AIR construit maintenant la position avec `nir_build_frag_coord`, qui suit la forme demandée par le pilote (RADV : `xy`, `z` et `1/w` séparés).

Les 13 écarts :

| Écart | Cas | Cause |
| --- | --- | --- |
| Filtrage anisotrope | 5 | Sa qualité est propre à chaque matériel : Metal ne la fixe pas. |
| PVRTC en stockage partagé | 6 | Le Mac charge et lit ces textures par le CPU. L'exécuteur passe par un blit, faute de mémoire d'invité sous Linux. C'est une limite de l'exécuteur, à revoir avec macOS invité. |
| Cibles `rgb10a2unorm` et `bgr10a2unorm` | 2 | Une unité de moins sur certains pixels. Pour ces formats, RADV fait sortir la couleur en flottant 16 bits (`ac_choose_spi_color_formats`), ce que son code présente comme requis avec RB+. |

## Ce qui reste pour J4

- Étendre l'exécuteur aux familles qui restent sous Linux : indirect command buffers (45 cas), tessellation (32), argument buffers (24), tables de fonctions (21), tuiles (19), vues de textures (15), mesh (14), et quelques familles plus petites (164 cas en tout).
- Compiler sur le Mac les bibliothèques `*_runtime_*`, que le Mac compile à partir du MSL pendant le cas (737 cas).
- Mesurer les alignements réels (lignes et décalages des surfaces linéaires) sur des images linéaires de RADV, pour remplacer les 256 octets estimés de `metal/amd.rs`.
