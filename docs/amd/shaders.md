# Les shaders de macOS compilés pour gfx1201 (J3)

Ce rapport mesure combien des shaders que macOS 27 embarque passent le compilateur Metal de Stela pour la Radeon RX 9070 XT (GFX1201), et d'où viennent les refus.

## Ce qui est mesuré, et ce qui ne l'est pas

`stela_metal_corpus_amd` compile chaque point d'entrée comme le fait le périphérique Metal : `stela_metal_compile`, avec les options NIR que RADV donne à son backend ACO (`ac_nir_set_options`) et les limites que RADV annonce sur la carte (`crates/stela-driver-gpu/src/amd/tests/metal_corpus_target.h`). Le résultat est le NIR que Stela remettrait à RADV.

Ce rapport ne mesure pas l'étape suivante : la compilation de ce NIR en code machine par RADV et ACO. Cette étape relève de J4, avec un vrai pipeline Vulkan sur RADV. La cible NVIDIA de l'outil s'arrête au même niveau, avec les options de NAK.

## Comment refaire la mesure

```sh
# 1. Compilateurs avec la cible AMD et le frontal AIR (LLVM 23)
CLANG_PATH=/usr/lib/llvm-23/bin/clang cargo xtask gpu compilers \
  --llvm-library /usr/lib/llvm-23/lib/libLLVM-23.so --llvm-config /usr/bin/llvm-config-23
# 2. AIR de la suite Metal, compilé et vérifié sur le Mac
cargo xtask gpu metal-suite mac --host USER@MAC
# 3. Suite hors ligne pour gfx1201, puis pour NVIDIA pour comparer
B=target/gpu/compilers-llvm-*/compiler
cargo xtask gpu metal-suite offline --compiler $B/stela_metal_corpus_amd --gpu gfx1201
cargo xtask gpu metal-suite offline --compiler $B/stela_metal_corpus
# 4. Bibliothèques Metal du système, copiées depuis le Mac
PATH=/usr/lib/llvm-23/bin:$PATH cargo xtask gpu metal-suite coverage --host USER@MAC
# 5. Les shaders de macOS (--all : toutes les bibliothèques hors AGX)
python3 -I xtask/scripts/amd-macos-shaders.py --tool $B/stela_metal_corpus_amd [--all]
python3 -I xtask/scripts/amd-macos-shaders.py --tool $B/stela_metal_corpus --sm 120 --all
```

## Mesure du 16 novembre 2026

Mac de référence : macOS 27.2.1 et Xcode, sur une puce Apple M3 Pro. Compilateur : Mesa 26.2.5 de Stela et LLVM 23.1.6.

### Corpus du premier démarrage

| Bibliothèque | Compilés | Refusés |
| --- | --- | --- |
| SkyLight (WindowServer) `SkyLightShaders.air64.metallib` | 33 | 0 |
| QuartzCore (CoreAnimation) `default.metallib` | 240 | 0 |
| SetupAssistantSupportUI (assistant de configuration) | 6 | 0 |
| CoreImage `default`, `ci_ubershader`, `ubershader_archive_bin` | 1 950 | 0 |
| Metal `default`, `MTLECBE`, `MTLMeshShaderEmulator` | 12 | 0 |
| Metal `MTLBVHBuilder` (construction des structures du ray tracing) | 189 | 38 |
| CoreImage `coreui_archive_bin` (archive assemblée à l'exécution) | 0 | 120 |

Tout ce que WindowServer, CoreAnimation et l'assistant de configuration dessinent compile pour gfx1201. Les deux refus ne concernent pas le démarrage :
- `MTLBVHBuilder` sert au ray tracing ;
- l'archive `coreui_archive_bin` porte la métadonnée `air.signature` des fonctions assemblées à l'exécution (*stitching*), que le frontal ne lit pas encore.

### Toutes les bibliothèques de macOS (hors pilotes AGX)

255 bibliothèques, 57 401 points d'entrée, en comptant à part chaque module des archives binaires :

| Cible | Compilés |
| --- | --- |
| gfx1201 (RADV) | 44 082 (77 %) |
| SM 120 (NAK) | 44 082 (77 %) |

**Aucun refus n'est propre à AMD** : les deux cibles acceptent et refusent exactement les mêmes points d'entrée. Tous les refus viennent du frontal AIR commun. Les plus fréquents :

| Refus | Nombre |
| --- | --- |
| `air.signature` (fonctions assemblées à l'exécution) | 6 720 |
| arithmétique autre que half et float (bfloat) | 878 |
| accès mémoire volatile, ordonné ou non lié | 815 |
| type de sortie de rendu non pris en charge | 730 |
| stockage d'un pointeur dans un espace d'adressage | 554 |
| structures d'accélération du ray tracing en argument | 524 |

Les plus gros consommateurs de refus sont des cadres du visionOS et de la 3D (Vista, CoreRE, RemoteCompositorServices, CompositorServices) et la bibliothèque d'apprentissage automatique MLX (GESS). Aucun ne sert au premier démarrage.

### Suite Metal de Stela

- Sur le Mac : 14 146 cas réussis, 0 échec, 16 non pris en charge.
- Hors ligne : 5 902 fonctions acceptées et 6 244 refusées, à l'identique pour gfx1201 et pour NVIDIA. 6 218 des refus viennent de la même limite du frontal : les 12 bibliothèques `integer_*` ont 592 noyaux dans un seul module, au-delà de `AIR_MAX_METADATA_OPERANDS`.

## Corrections faites en route

Dans l'outillage d'upstream, sans rapport avec AMD :

- `metal-suite offline` passait `--entry` et `--nir` à `stela_metal_corpus`, qui ne les connaissait pas : l'outil prenait ces arguments pour des fichiers. Les deux options existent maintenant.
- `cargo xtask gpu compilers` refusait trois choses courantes sous Linux :
  - les archives minces (*thin*) qu'écrit GNU ar pour Meson ;
  - `libdl.a`, vide depuis glibc 2.34 ;
  - `libm.a`, qui est un script pour l'éditeur de liens GNU.
- Sous Ubuntu 24.04, avec LLVM 18 et LLVM 23 installés ensemble, bindgen charge la libclang 23 mais les en-têtes de clang 18. Il faut `CLANG_PATH=/usr/lib/llvm-23/bin/clang`.
