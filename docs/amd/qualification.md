# Qualification de la RX 9070 XT sous Linux (J2)

Sortie de `stela-amd-host` sur la carte de référence, archivée telle quelle, puis analysée.

## Machine

| | |
| --- | --- |
| Date | 15 novembre 2026 |
| Carte | ASRock RX 9070 XT (PCI `1002:7550`, sous-système `1849:5417`), `0000:07:00.0` |
| Noyau | Linux 6.14.0-37-generic (Ubuntu 24.04), pilote amdgpu |
| Mesa | 25.2.8-0ubuntu0.24.04.4 (RADV GFX1201, Vulkan 1.4.318) |
| libdrm_amdgpu | 2.4.125 |
| Rust | 1.99.1 (`rust-toolchain.toml`) |
| Resizable BAR | active (BAR0 de 16 Gio) |
| IOMMU | actif, la carte seule dans le groupe 19, sa fonction audio dans le groupe 20 |

## Commande

```sh
cargo build -p stela-amd-host --release
VK_DRIVER_FILES=/usr/share/vulkan/icd.d/radeon_icd.json ./target/release/stela-amd-host
```

Code de sortie : 1, parce que la carte n'était pas encore admise.

## Première sortie

```text
WARNING: radv is not a conformant Vulkan implementation, testing use only.
Vulkan device 0: AMD Radeon RX 9070 XT (RADV GFX1201) (vendor 1002, device 7550)
  driver: MesaRadv, Vulkan 1.4.318
  memory: 16304 MiB device-local, 16304 MiB reached by the CPU
  backend requirements missing: dynamicPrimitiveTopologyUnrestricted
  dma-buf import: yes
  PCI: 0000:07:00.0
  graphics core: GC 12.0.1
  admission: GFX1201 Navi 48 (Radeon RX 9070 series), PCIe gen 5 x16
  Metal architecture: GFX1201
  Metal GPU families: [Apple1, Apple2, Apple3, Apple4, Apple5, Apple6, Apple7, Common1, Common2, Common3, Mac1, Mac2, MacCatalyst1, MacCatalyst2, Metal3]
  Metal gaps (8):
    - 4:2:2 pixel formats (Driver)
    - Depth24Unorm_Stencil8 (Driver)
    - primitive motion blur (Driver)
    - placement sparse resources (AppleArchitecture)
    - tile-dispatch counter sampling (AppleArchitecture)
    - sparse multisample textures (Driver)
    - Apple GPU families from Apple8 (AppleArchitecture)
    - GPU family Metal4 (AppleArchitecture)

RESULT: no AMD GPU qualifies yet; the lines above say why.
```

## Analyse

Ce qui est atteint :

- Avant le correctif ci-dessous, 19 des 20 fonctions requises étaient présentes, et l'import dma-buf aussi.
- La table `amdgpu_chip` reconnaît bien la carte (GC 12.0.1, Navi 48, PCIe 5.0 x16).
- Metal3 et Apple7 sont atteints dès maintenant. C'est l'objectif du jalon J2.
- Les 16 Gio de VRAM sont entièrement accessibles au processeur, grâce à la Resizable BAR.

### Le blocage `dynamicPrimitiveTopologyUnrestricted`, levé

RADV renvoie `false` pour cette propriété de `VK_EXT_extended_dynamic_state3`. Le backend l'exigeait. Sans elle, Vulkan ne change la topologie d'un dessin qu'à l'intérieur de la classe (points, lignes ou triangles) avec laquelle le pipeline a été créé.

Stela crée chaque pipeline avec la topologie de sa classe Metal (`class_topology` dans `metal/pipeline.rs`), puis fixe la topologie exacte au moment du dessin. Le seul cas qui sort de la classe est un pipeline dont la classe n'est pas précisée (`MTLPrimitiveTopologyClassUnspecified`, la valeur par défaut sous macOS). Stela le crée en `TriangleList`, alors que Metal permet ensuite d'y dessiner des points ou des lignes.

Le correctif :

- `dynamicPrimitiveTopologyUnrestricted` ne fait plus partie des exigences (`vulkan/backend.rs`, 19 fonctions requises au lieu de 20).
- Quand la propriété manque, un pipeline de classe non précisée est marqué `topology_variants` (`RenderPipelinePlan`).
- Un dessin de points ou de lignes sur ce pipeline lie une variante créée avec sa classe. Les variantes sont compilées à la première utilisation et gardées par classe, avec le mécanisme qui sert déjà aux échantillonneurs bicubiques (`Device::render_variant`, `Resolver::bicubic_render_pipeline`). Les dessins de triangles gardent le pipeline d'origine.
- Limite connue : les commandes encodées par le GPU (indirect command buffers) ne lient pas de variantes. Ces pipelines ne leur sont donc pas proposés : `supportIndirectCommandBuffers` y est sans effet, et les exécuter depuis un indirect command buffer donne une erreur explicite. Si macOS en a besoin, il faudra un jeu d'exécution par classe de topologie.
- Sur NVIDIA, où la propriété est présente, rien ne change.

Sortie de `stela-amd-host` après le correctif :

```text
Vulkan device 0: AMD Radeon RX 9070 XT (RADV GFX1201) (vendor 1002, device 7550)
  driver: MesaRadv, Vulkan 1.4.318
  memory: 16304 MiB device-local, 16304 MiB reached by the CPU
  backend requirements: all 19 present
  primitive topology: restricted to the pipeline's class; point and line draws of unspecified pipelines compile a variant, and GPU-encoded commands cannot switch to those pipelines
  dma-buf import: yes
  ...
RESULT: an AMD GPU qualifies for Stela's Metal device over RADV.
```

Code de sortie : 0.

### Les manques Metal

| Manque | Origine | Commentaire |
| --- | --- | --- |
| Formats 4:2:2 | Pilote | À vérifier : formats `G8B8G8R8_422` et `B8G8R8G8_422` dans RADV |
| Depth24Unorm_Stencil8 | Pilote | Attendu : les GPU AMD n'ont pas de D24S8. Metal le rend facultatif sur Mac (`depth24Stencil8PixelFormatSupported`) |
| Flou de mouvement des primitives | Pilote | Lié au ray tracing, sans effet sur le démarrage |
| Textures sparse multi-échantillonnées | Pilote | RADV n'expose pas `sparseResidency*Samples` |
| Ressources sparse en placement, comptage des tile dispatch, Apple8+, Metal4 | Architecture Apple | Hors de portée d'un GPU non Apple, comme sur NVIDIA |

### Alignements que renvoie RADV

Lus avec `vulkaninfo` sur la même carte :

| Limite | Valeur |
| --- | --- |
| `minStorageBufferOffsetAlignment` | 4 |
| `minUniformBufferOffsetAlignment` | 4 |
| `minTexelBufferOffsetAlignment` | 4 |
| `optimalBufferCopyOffsetAlignment` | 1 |
| `nonCoherentAtomSize` | 64 |
| `sparseAddressSpaceSize` | 0xfffffffc |
| `residencyStandard2DBlockShape` | true |

Ces valeurs ne disent rien de l'alignement des lignes de surfaces linéaires : `LINEAR_ROW_ALIGNMENT` et `LINEAR_OFFSET_ALIGNMENT` dans `metal/amd.rs` dépendent d'addrlib. Il faudra les mesurer avec `vkGetImageSubresourceLayout` sur une image linéaire, une fois le mode `--device` en place (J4).
