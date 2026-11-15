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

Code de sortie : 1, parce que la carte n'est pas encore admise.

## Sortie

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

- 19 des 20 fonctions requises sont présentes, et l'import dma-buf aussi.
- La table `amdgpu_chip` reconnaît bien la carte (GC 12.0.1, Navi 48, PCIe 5.0 x16).
- Metal3 et Apple7 sont atteints dès maintenant. C'est l'objectif du jalon J2.
- Les 16 Gio de VRAM sont entièrement accessibles au processeur, grâce à la Resizable BAR.

### Le seul blocage : `dynamicPrimitiveTopologyUnrestricted`

RADV renvoie `false` pour cette propriété de `VK_EXT_extended_dynamic_state3`. Le backend exige qu'elle soit vraie : `crates/stela-driver-gpu/src/metal/vulkan/backend.rs` (liste des `requirement`).

Stela crée chaque pipeline avec la topologie de sa classe Metal (`class_topology` dans `metal/pipeline.rs`), puis fixe la topologie exacte au moment du dessin. Tant qu'un dessin reste dans la classe du pipeline (liste ou bande de triangles pour un pipeline de triangles), le Vulkan standard le permet sans cette propriété. Le problème vient de la classe `Unspecified` : le pipeline est créé en `TriangleList`, alors que Metal autorise ensuite des points ou des lignes.

Il reste deux pistes, à trancher dans la suite de J2 :

1. Côté Stela : rendre l'exigence conditionnelle. Sans la propriété, créer une variante du pipeline par classe de topologie pour les pipelines `Unspecified`, à la demande au moment du dessin.
2. Côté RADV : vérifier si une version plus récente de Mesa l'expose sur GFX12, où le matériel fixe la topologie dynamiquement.

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
