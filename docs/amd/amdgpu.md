# amdgpu dans Stela (J6)

Le but : faire démarrer la RX 9070 XT par Stela lui-même, avec le pilote amdgpu de Linux compilé tel quel dans l'environnement Linux de Stela (`stela-linux`). C'est la méthode déjà suivie pour nouveau sur NVIDIA.

## Où en est le portage (16 novembre 2026)

- **Sources.** amdgpu, son affichage (DC), sa gestion de l'énergie et ses en-têtes sont dans l'arbre Linux épinglé (`third_party/linux.sparse`). D'amdkfd, l'API de calcul de Linux dont Stela ne se sert pas, seuls les en-têtes inclus par amdgpu sont gardés.
- **Kbuild.** Les makefiles d'amdgpu demandent bien plus de Make que le lecteur de Stela n'en accepte (`build/kbuild.rs`). `xtask/scripts/amd-kbuild.py` les évalue donc avec GNU Make, sous la configuration de Stela, et écrit `crates/stela-driver-gpu/build/amdgpu-kbuild.json` : 892 objets (516 pour l'affichage, 269 pour amdgpu, 81 pour l'énergie, 26 pour RAS), leurs options, et celles des 70 objets de calcul en virgule flottante de DC.
- **Configuration.** Elle vise les RDNA 4 de bureau. Le cœur d'affichage est inclus, avec son code en virgule flottante, qu'exige DCN. Le reste suit le Kconfig de Stela, sans ACPI comme pour nouveau. Sont exclus GCN 1 et 2 (SI, CIK), KFD, userptr (HMM), ACP, ISP, debugfs, procfs, perf et les ioctls 32 bits.
- **Recensement** (fonctionnalité Cargo `amdgpu-census`, `build/amdgpu.rs`). Chaque source est compilée avec les options de l'environnement et celles d'amdgpu, puis les symboles importés sont comparés à ce que l'environnement définit. Résultat :
  - **889 sources sur 892 compilent** ;
  - 3 attendent un service : `asm/fpu.h` (sauvegarde de l'état flottant), `generic_handle_domain_irq` et `pfn_to_nid` ;
  - **105 symboles importés** ne sont pas encore fournis. Une trentaine sont ceux d'amdgpu lui-même, dans les 3 sources en attente et dans le module `amdxcp`, à ajouter. Le reste se range en familles de services noyau (liste complète dans `amdgpu-census.txt`) :
    - PCI : redimensionnement des BAR, état sauvegardé, capacités étendues, bande passante, opérations atomiques ;
    - assistants DRM à ajouter à la compilation : buddy, suballoc, DSC, HDCP, client ;
    - et `kfifo`, `mempool`, `devres`, sémaphores, hwmon, rétroéclairage, temps, `sscanf` et `gcd`, et l'activation des interruptions.

```sh
python3 -I xtask/scripts/amd-kbuild.py                  # après un changement de l'épinglage Linux
cargo build -p stela-driver-gpu --features amdgpu-census  # écrit OUT_DIR/amdgpu-census.txt
```

## Suite

1. Fournir les 3 services qui manquent à la compilation, puis les familles de services une à une, jusqu'à ce que le recensement n'ait plus de symbole ouvert. C'est alors un vrai port, sur le modèle de `build/nouveau.rs`, avec sa liste `OPEN` vérifiée à chaque build.
2. Enregistrer le pilote au démarrage (`amdgpu.rs`, `amd.rs`), provisionner les firmwares de la Navi 48, et faire les premiers essais en VM : IP discovery, PSP, SMU, MES, SDMA, puis le premier paquet de commandes.

## Travailler pendant les essais en VM

La machine n'a qu'une carte graphique (le Ryzen 5 5600X n'a pas de GPU intégré). La prêter à la VM arrête le serveur X, et avec lui toutes les applications graphiques, dont Claude. Ne couper que la vidéo est impossible avec une seule carte. Une session Claude Code lancée dans `tmux` depuis un terminal survit, elle : `xtask/scripts/amd-agent.sh` la démarre (paquet `tmux` requis). Logind garde les processus d'un utilisateur dont la session se termine (`KillUserProcesses=no`, le défaut de Mint). Le travail continue donc pendant l'essai, et le bureau revient à la fin. On peut suivre la session depuis le Mac ou un téléphone, par `ssh -t … tmux attach -t stela` (avec `openssh-server`) ou par le Remote Control de Claude.

Une deuxième carte graphique, même modeste, dans un autre emplacement PCIe, garderait le bureau entier pendant les essais.
