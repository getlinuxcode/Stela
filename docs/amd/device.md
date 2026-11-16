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
- **Topologie** : le même pipeline, sans classe de topologie, dessine une bande de lignes. Sur RADV, qui ne fournit pas `dynamicPrimitiveTopologyUnrestricted`, ce dessin passe par la variante « lignes » du correctif de [tylertms/Stela#158](https://github.com/tylertms/Stela/pull/158). Le correctif est donc vérifié sur le matériel.

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

`stela-amd-host --suite DIR` exécute des cas de la suite Metal sur ce périphérique, comme l'exécuteur du Mac (`runner.m`). Il prend les bibliothèques que le Mac a compilées (`DIR/plan/lib`) et écrit ses résultats là où `cargo xtask gpu metal-suite check` les lit (`DIR/results/linux-gfx1201`). Un cas déjà fait est sauté, ce qui permet de reprendre une exécution interrompue. Il couvre deux familles :

- les **calculs** qui se résument à une bibliothèque, une fonction, des buffers, une grille et un threadgroup (`suite.rs`) ;
- les **rendus** en un seul tracé, sur des textures 2D chargées depuis leurs fichiers, avec des attachements de couleur qui chargent ou effacent, du mélange, des masques d'écriture et des buffers par étage (`suite_render.rs`).

```sh
cargo xtask gpu metal-suite mac --host USER@MAC       # plan, bibliothèques et résultats de référence
sg kvm -c "./target/release/stela-amd-host --suite .stela/metal-suite"
cargo xtask gpu metal-suite check .stela/metal-suite/results/linux-gfx1201
```

Mesure du 16 novembre 2026, en 19 secondes :

| | Cas |
| --- | --- |
| Identiques au Mac, dans ses tolérances | 11 843 (11 727 calculs, 115 rendus, 1 cas d'information sur le périphérique) |
| Écarts | 2 (cibles `rgb10a2unorm` et `bgr10a2unorm`, voir plus bas) |
| Non pris en charge | 712 : 711 bibliothèques `*_runtime_*`, que le Mac compile à partir du MSL pendant le cas, et 1 cas de rendu avec des buffers d'un autre étage |
| Pas encore exécutés | 1 602 : profondeur, stencil, descripteurs de sommets, multi-échantillonnage, échantillonneurs, textures non 2D, séquences, blits, API… |

Corrections faites en route :

- **RADV ne savait pas compiler `sin`, `cos` et `tan`.** ACO refuse l'instruction NIR `fsin`, que RADV transforme d'habitude à la fin de son chemin SPIR-V (`nir_normalize_sin_cos`, `nir_lower_doubles`). Le NIR reçu de Stela ne passait pas par là : le patch `9001` lui applique ces deux passes. Au passage, l'erreur d'ACO faisait planter le processus au lieu de refuser le pipeline.
- **Largeur SIMD.** Stela prenait `maxSubgroupSize` comme `threadExecutionWidth`. Ça donnait 32 sur NVIDIA, mais 64 sur RADV, qui compile en 32 ou en 64 fils. Or le code Metal écrit pour les GPU Apple compte sur des simdgroups de 32 : `simd_shuffle_rotate_*`, `simdgroup_barrier` et les fonctions `simd_*` divergeaient. Stela prend maintenant 32 dès que le pilote le permet (`capabilities::simd_width`).
- **Position du fragment.** Le frontal AIR émettait `nir_load_frag_coord`, qu'ACO ne sait pas compiler : RADV demande la forme séparée (`xy`, `z`, `1/w`). Le frontal construit maintenant la position avec `nir_build_frag_coord`, qui suit la forme du pilote. Sur NVK, dont la forme est simple, le NIR reste le même.

Écart connu : sur une cible `rgb10a2unorm` ou `bgr10a2unorm`, la carte donne parfois une unité de moins que le Mac (40 à 54 pixels sur 256). Pour ces formats, RADV fait sortir la couleur du shader en flottant 16 bits (`ac_choose_spi_color_formats`), et le code de RADV présente ce choix comme requis avec RB+. Le flottant 16 bits a juste assez de précision pour 10 bits, mais pas pour arrondir comme Apple.

## Ce qui reste pour J4

- Étendre l'exécuteur au reste du rendu (profondeur, stencil, sommets, multi-échantillonnage, échantillonneurs), puis aux textures, séquences, blits et cas d'API. Ensuite, compiler sur le Mac les bibliothèques `*_runtime_*`, pour les faire passer elles aussi.
- Mesurer les alignements réels (lignes et décalages des surfaces linéaires) sur des images linéaires de RADV, pour remplacer les 256 octets estimés de `metal/amd.rs`.
