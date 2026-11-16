# amdgpu dans Stela (J6)

Le but : faire démarrer la RX 9070 XT par Stela lui-même, avec le pilote amdgpu de Linux compilé tel quel dans l'environnement Linux de Stela (`stela-linux`). C'est la méthode déjà suivie pour nouveau sur NVIDIA.

## Où en est le portage (17 novembre 2026)

- **Sources.** amdgpu, son affichage (DC), sa gestion de l'énergie et ses en-têtes sont dans l'arbre Linux épinglé (`third_party/linux.sparse`). D'amdkfd, l'API de calcul de Linux dont Stela ne se sert pas, seuls les en-têtes inclus par amdgpu sont gardés.
- **Kbuild.** Les makefiles d'amdgpu demandent bien plus de Make que le lecteur de Stela n'en accepte (`build/kbuild.rs`). `xtask/scripts/amd-kbuild.py` les évalue donc avec GNU Make, sous la configuration de Stela, et écrit `crates/stela-driver-gpu/build/amdgpu-kbuild.json` : 892 objets (516 pour l'affichage, 269 pour amdgpu, 81 pour l'énergie, 26 pour RAS), leurs options, et celles des 70 objets de calcul en virgule flottante de DC.
- **Configuration.** Elle vise les RDNA 4 de bureau. Le cœur d'affichage est inclus, avec son code en virgule flottante, qu'exige DCN. Le reste suit le Kconfig de Stela, sans ACPI comme pour nouveau. Sont exclus GCN 1 et 2 (SI, CIK), KFD, userptr (HMM), ACP, ISP, debugfs, procfs, perf et les ioctls 32 bits.
- **Recensement** (fonctionnalité `amdgpu-census`). Les 910 sources (avec `scan.c`) compilent, et plus aucun service noyau ne manque (`amdgpu-census.txt`).
- **Port lié** (fonctionnalité `amdgpu-port`, `build/amdgpu.rs`) :
  - les objets sont archivés dans `stela-amdgpu`, compilés en `-O2` comme Kbuild ;
  - l'image `stela_linux_amdgpu_image` enchaîne les enregistrements de l'environnement, puis le `module_init` de l'allocateur buddy (`drivers/gpu/buddy.c`), puis celui d'amdgpu ;
  - comme pour nouveau, le build échoue si les services qu'amdgpu atteint sans qu'on les serve changent (liste `OPEN`). Il en reste 11 : les 9 que nouveau accepte déjà (shmem, chemins VFS, liens d'appareils), et `pci_wake_from_d3` et `pci_wait_for_pending_transaction` (veille à l'exécution, retrait). La veille à l'exécution est coupée au démarrage (`amdgpu.runpm=0`, `src/amdgpu.rs`), puisque Stela ne met jamais le GPU en veille.
- **Ce que le fork fournit** (`crates/stela-driver-gpu/src/amd/linux/`), sans toucher aux sources d'amdgpu :
  - `include/stela/amdgpu.h`, inclus avant chaque source : le domaine d'interruptions des appareils enfants (ACP, ISP), qu'amdgpu ne crée pas pour RDNA 4, et `pfn_to_nid` (un seul nœud mémoire) ;
  - `include/asm/fpu.h` : la section flottante du noyau. Les changements de contexte de Stela sauvent déjà l'état x87/SSE (`FXSAVE64`), et une fibre ne cède le processeur que par un appel de sa part ;
  - `services.c` :
    - pas de sysfs, de hwmon ni de rétroéclairage (panneau interne seulement) ;
    - messages non limités et horloges ;
    - les bus I2C gérés (`devm_i2c_add_adapter`) ;
    - le masquage des interruptions par niveaux (`disable_irq`, `enable_irq`), utilisé pendant une réinitialisation du GPU ;
    - l'arrêt de Stela à la place d'un redémarrage d'urgence ;
    - le refus des partitions de calcul d'amdxcp (GPU Instinct seulement) ;
  - `pci.c` : capacités et état du lien lus dans l'espace de configuration, un vecteur MSI comme Stela le donne, et refus de ce qui reconfigure le bus (redimensionnement de BAR, réinitialisation de fonction, atomiques vers la racine). Le BAR de la VRAM reste tel que le firmware l'a dimensionné, c'est-à-dire en entier quand Resizable BAR est activé ;
  - `scan.c` : `sscanf`, avec les conversions de Linux.
- **Dans l'environnement** (`stela-linux`, `kernel/device.c`) : les groupes de ressources gérées (`devres_open_group`, `devres_close_group`, `devres_release_group`), qui vivent à côté de la liste des actions gérées. Ils sont testés par `c_tests::linux_device`.
- **Au démarrage** (fonctionnalité `amdgpu` de `stela-boot`) :
  - `engine/gpu_amd.rs` prend chaque Radeon RDNA 4 comme `engine/gpu.rs` prend une NVIDIA : même passage à froid (`cold.rs` accepte maintenant le fabricant AMD) ;
  - différences : les registres sont au BAR 5, il n'y a pas de PMC_BOOT_0, et le catalogue de firmwares est vide pour l'instant, donc les demandes d'amdgpu échouent comme des fichiers absents ;
  - l'image d'amdgpu démarre à la place de celle de nouveau.
- **Mode essai** (`--gpu-trial`) : démarre le GPU même sans macOS, ce que la VM ne peut pas fournir. Sans carte, la VM le vérifie déjà : `GPU TRIAL started=false`.

```sh
python3 -I xtask/scripts/amd-kbuild.py                  # après un changement de l'épinglage Linux
cargo build -p stela-driver-gpu --features amdgpu-census  # écrit OUT_DIR/amdgpu-census.txt
cargo build -p stela-driver-gpu --features amdgpu-port    # le port lié, avec sa liste OPEN
xtask/scripts/amd-gpu-trial.sh build                    # stela-amdgpu.efi
sudo systemd-run --unit=stela-amd-gpu --collect \
  --setenv=STELA_VFIO_USER=$USER "$PWD/xtask/scripts/amd-gpu-trial.sh" run
```

Le dernier essai écrit `.stela/vm/gpu-trial/report.txt` : les lignes GPU du journal série, et l'état de la carte avant et après.

## Suite

1. Premier essai en VM avec la carte prêtée : voir jusqu'où amdgpu va (passage à froid, IP discovery, puis l'échec attendu au premier firmware demandé).
2. Firmwares de la Navi 48 : les épingler (recettes comme pour le GSP de NVIDIA) et les fournir à amdgpu par son catalogue. Ensuite PSP, SMU, MES, SDMA, puis le premier paquet de commandes.

## Travailler pendant les essais en VM

La machine n'a qu'une carte graphique (le Ryzen 5 5600X n'a pas de GPU intégré). La prêter à la VM arrête le serveur X, et avec lui toutes les applications graphiques, dont Claude. Ne couper que la vidéo est impossible avec une seule carte. Une session Claude Code lancée dans `tmux` depuis un terminal survit, elle : `xtask/scripts/amd-agent.sh` la démarre (paquet `tmux` requis). Logind garde les processus d'un utilisateur dont la session se termine (`KillUserProcesses=no`, le défaut de Mint). Le travail continue donc pendant l'essai, et le bureau revient à la fin. On peut suivre la session depuis le Mac ou un téléphone, par `ssh -t … tmux attach -t stela` (avec `openssh-server`) ou par le Remote Control de Claude.

Une deuxième carte graphique, même modeste, dans un autre emplacement PCIe, garderait le bureau entier pendant les essais.
