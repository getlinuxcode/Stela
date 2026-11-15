# Feuille de route : Stela pour AMD Radeon RDNA 4

Ce fork ajoute à Stela l'accélération Metal sur les Radeon RDNA 4 (RX 9070 / 9070 XT, Navi 48, et RX 9060, Navi 44). Le projet d'origine, [tylertms/Stela](https://github.com/tylertms/Stela), reste centré sur NVIDIA. Ce fork le suit et n'y ajoute que la partie AMD.

## Principe

Stela gère NVIDIA en trois couches. AMD suivra exactement le même modèle :

| Couche | NVIDIA (upstream) | AMD (ce fork) |
| --- | --- | --- |
| Pilote noyau, compilé sur la couche Linux de Stela | nouveau + firmware GSP | amdgpu + firmwares PSP/SMU/MES/DCN |
| Pilote Vulkan, chargé dans le même processus | NVK | RADV |
| Compilateur de shaders | NAK | ACO (AIR→NIR→ACO déjà présent pour gfx1201) |
| Couche Metal de Stela, au-dessus de Vulkan | commune | commune |
| macOS : kext, bundle Metal, modèle du DCP | commun | commun, rien à refaire |

Sans périphérique Metal, macOS ne démarre pas sous Stela : WindowServer s'arrête et Stela interrompt le démarrage. **Le premier démarrage de macOS sur AMD sera donc accéléré, sinon il n'aura pas lieu.** Tout ce qui précède se construit et se teste sous Linux, puis dans l'étape de démarrage de Stela, sans macOS.

## Règles du fork

- Le code AMD vit dans des fichiers à lui (`amdgpu_chip.rs`, `metal/amd/`, `stela-amd-host/`, et plus tard `amdgpu.rs`, `xtask/src/gpu/radv.rs`, `third_party/patches/amdgpu/`). On touche le moins possible aux fichiers d'upstream.
- `upstream/main` est fusionné régulièrement par merge, jamais par rebase, avec `git rerere` activé.
- Les correctifs qui ne concernent pas AMD repartent vers upstream sous forme de petites PR.
- Matériel de référence : une seule RX 9070 XT, sans carte NVIDIA. macOS ne démarre pas avant le jalon 7.

## Développement en VM

Tout le développement se fait depuis Linux, sans redémarrer le PC à chaque essai :

| Jalons | Où |
| --- | --- |
| J2 à J5 | Linux natif, la carte sous amdgpu + RADV |
| J6 et J7 | Stela dans une VM QEMU/KVM, la carte prêtée par VFIO |

Upstream sait déjà le faire : `cargo xtask boot run --kvm --vfio-gpu`. La VM garde un écran VGA émulé pour le firmware, donc l'écran de démarrage de Stela et ses messages restent visibles même quand la Radeon n'affiche rien. Les journaux arrivent directement sous Linux.

```sh
# État de la carte : pilote, groupe IOMMU, autres sorties d'affichage
xtask/scripts/vfio-gpu.sh status
# Un essai : la carte quitte amdgpu, va dans la VM, puis revient
sudo xtask/scripts/vfio-gpu.sh run --stop-desktop --seconds 1800 -- \
  cargo xtask boot run --kvm --vfio-gpu 0000:03:00.0 --usb-image stick.img
```

- `vfio-gpu.sh` choisit une NVIDIA, sinon une Radeon RDNA 4 (0x7550, 0x7590). Il n'utilise jamais la puce graphique intégrée d'un Ryzen. `STELA_VFIO_GPU=0000:03:00.0` force une carte précise.
- **Affichage de l'hôte** : si l'écran est branché sur la 9070 XT, `--stop-desktop` coupe le bureau Linux pendant l'essai. Il vaut mieux brancher l'écran sur la carte mère (puce graphique intégrée du processeur) et laisser la 9070 XT sans écran, ou suivre la VM à distance (VNC, SSH).
- **Le bug de réinitialisation des Radeon** : la Navi 48 se réinitialise mal en VFIO. Le script laisse amdgpu prendre la carte au démarrage et ne la détache qu'au moment de l'essai, comme le recommandent les retours d'expérience. Il ne la réserve jamais à vfio-pci dès le démarrage. Le script désactive aussi la réinitialisation FLR (`reset_method` vide), et QEMU réinitialise alors le slot. Ce choix est à valider sur la Navi 48 : certains utilisateurs ont dû interdire aussi la réinitialisation du bus. Un pilote amdgpu en cours de mise au point laissera parfois la carte dans un état que seul un redémarrage de l'hôte répare.
- Activer l'IOMMU (`amd_iommu=on`, souvent actif par défaut) et la Resizable BAR dans le firmware de la carte mère.

## Où on en est (15 novembre 2026)

Fait depuis Windows, sans la carte :

- Fork `getmowbot/Stela` : les patchs J0 et J2, `vfio-gpu.sh` et cette feuille de route sont dans `main`. Le remote `upstream` pointe sur `tylertms/Stela`, avec `git rerere` activé dans le clone Windows. À refaire dans chaque nouveau clone.
- `amd-ci.yml` passe sur GitHub : rustfmt, clippy et tests de `stela-driver-gpu`, `stela-boot-core` et `stela-amd-host`. Le code AMD compile, mais n'a encore jamais tourné sur la carte.
- `amd-sync-upstream.yml` tourne chaque jour à 4 h 17 UTC. GitHub Actions a le droit d'ouvrir des PR sur le fork. En cas de conflit, la fusion se fait à la main dans `sync/upstream`.
- Les commits du fork portent l'identité `Stela contributor <contributor@example.invalid>`, comme les patchs.

Fait sous Linux (Ubuntu 24.04, noyau 6.14, Mesa 25.2.8) :

- Le clone Linux a son remote `upstream`, `git rerere` activé et l'identité des commits du fork. Rust 1.99.1 est installé par rustup.
- J2 : `stela-amd-host` a tourné sur la 9070 XT, et sa sortie est archivée dans `docs/amd/qualification.md`. Seul manque au départ : `dynamicPrimitiveTopologyUnrestricted`, que RADV ne fournit pas. Ce n'est plus une exigence : sans cette propriété, les pipelines Metal sans classe de topologie compilent une variante pour les points et une pour les lignes. **La carte est maintenant admise** (code de sortie 0), avec l'import dma-buf, Metal3 et Apple7. Limite : ces pipelines ne sont pas proposés aux indirect command buffers.
- J1 : `vfio-gpu.sh status` reconnaît la carte (IOMMU et Resizable BAR actifs, groupes propres). Il n'y a pas d'autre sortie d'affichage : l'écran est branché sur la 9070 XT et le processeur n'a pas de puce graphique active.

À faire ensuite, dans cet ordre :

1. J2 : les manques Metal sont examinés, et aucun ne se règle côté Stela. Reste à ouvrir la PR upstream du correctif sur la topologie (branche `pr/topology-variants`, prête et testée sur la base d'upstream).
2. J3 : `cargo xtask gpu fetch` s'arrête faute du paquet `python3.12-venv`. Une fois ce paquet installé, construire les compilateurs avec la cible AMD : `cargo xtask gpu compilers`, avec `--llvm-library` et `--llvm-config` pour le frontal AIR, qui demande LLVM 23. La sonde AIR→ACO existe déjà dans `crates/stela-driver-gpu/src/amd/tests/air_aco.c`.
3. J3 : la suite `metal-suite offline` lit l'AIR qu'un Mac compile (`metal-suite mac`), et `coverage` lit les metallibs système d'un Mac. Sans Mac, il faudra les extraire d'une image de macOS 27.
4. J1, dernier point : le Ryzen 5 5600X n'a pas de puce graphique, et la 9070 XT est la seule sortie d'affichage. Avant le premier essai en VM, il faut ajouter une petite carte graphique pour l'écran de Linux, ou lancer l'essai par SSH et suivre la VM par VNC. Avec `--stop-desktop`, le bureau Linux s'arrête pendant l'essai. Noter ensuite combien d'essais d'affilée passent sans redémarrer l'hôte.

## Jalons

### J0 : Fondations ✅

- [x] Analyse de l'architecture d'upstream (nouveau + NVK + couche Metal sur Vulkan).
- [x] `amdgpu_chip` : table RDNA 4 indexée par la version du cœur graphique (GC 12.0.0 / 12.0.1), avec les identifiants PCI 0x7590 / 0x7550.
- [x] `metal::amd` : admission d'un GPU dédié sous RADV avec un cœur RDNA 4, et valeurs de plateforme (alignements, pages sparse de 64 Ko).
- [x] `metal::amd::open` : ouverture du service Metal en réutilisant la partie commune avec NVIDIA.
- [x] Écran de démarrage : la carte est nommée (« AMD Navi 48 (Radeon RX 9070 series) ») et le message explique pourquoi elle n'est pas encore utilisée.

### J1 : Infrastructure du fork

- [x] Créer le fork et y appliquer les patchs J0 et J2 (fusionnés dans `main`).
- [ ] Workflow GitHub Actions quotidien (`amd-sync-upstream.yml`, en place et lancé une fois sans rien à fusionner ; reste à voir une vraie fusion passer) : merge de `upstream/main` dans `sync/upstream`, tests, puis PR automatique si tout passe.
- [x] CI propre au fork (`amd-ci.yml`) : `cargo test` sur `stela-driver-gpu` (fonctionnalités `metal`, `metal-vulkan`), `stela-boot-core` et `stela-amd-host`, plus clippy et rustfmt.
- [x] `vfio-gpu.sh` sait prêter une Radeon RDNA 4 à la VM.
- [ ] Premier lancement de Stela en VM avec la 9070 XT prêtée : l'écran de démarrage nomme la carte et la rend proprement à la fin. Mesurer comment la réinitialisation se comporte sur plusieurs essais d'affilée.

**Terminé quand** une synchronisation avec upstream passe la CI sans intervention.

### J2 : Qualification sous Linux 🟡

- [x] `metal::native::survey` : examine chaque périphérique Vulkan comme le ferait le backend, sans le créer.
- [x] `stela-amd-host` : lit la version du cœur graphique et le lien PCIe dans sysfs, admet la carte et calcule le profil Metal (familles de GPU, et ce qui manque avec sa cause).
- [x] Lancer l'outil sur la 9070 XT (Linux 6.12+, Mesa récent) et archiver sa sortie dans `docs/amd/qualification.md`.
- [x] Lever le blocage `dynamicPrimitiveTopologyUnrestricted` : variantes de pipeline par classe de topologie quand le pilote ne fournit pas la propriété.
- [x] Traiter chaque manque Metal attribué au pilote : tous sont des limites connues (RADV, matériel AMD, ou Stela pour tous les GPU), voir `docs/amd/qualification.md`.
- [ ] Proposer le correctif sur la topologie à upstream (branche `pr/topology-variants`, prête).
- [ ] Remplacer les alignements estimés (256 octets) par les valeurs que RADV renvoie réellement.

**Terminé quand** les fonctions requises sont présentes (19 depuis le correctif sur la topologie), l'import dma-buf aussi, et Metal3 + Apple7 sont atteints sur la vraie carte. C'est atteint depuis le 15 novembre 2026. Restent les manques Metal et les alignements.

### J3 : Compiler les shaders de macOS pour gfx1201

Sans GPU nécessaire.

- [ ] Ajouter la cible AMD gfx1201 à `cargo xtask gpu metal-suite offline`.
- [ ] Étendre `coverage` à ce que le chemin ACO ne sait pas encore traduire.
- [ ] Extraire de l'image macOS 27 les bibliothèques Metal de WindowServer, CoreAnimation, QuartzCore et de l'assistant de configuration.
- [ ] Les compiler toutes pour gfx1201 et ouvrir un ticket pour chaque échec.

**Terminé quand** tous les shaders de ce corpus compilent pour gfx1201.

### J4 : La couche Metal de Stela tourne sur RADV sous Linux

- [ ] Construire les compilateurs de Stela avec la cible AMD (`cargo xtask gpu fetch`, Mesa et ses patches, option `compilers=amd`).
- [ ] Mode `--device` de `stela-amd-host` : ouvrir le vrai périphérique Metal (`NativeBackend` + `AirCompiler`) sur RADV, avec des noyaux internes compilés pour gfx1201.
- [ ] Premier calcul : addition de deux buffers, résultat vérifié.
- [ ] Premier rendu : un triangle dans une texture, comparé à une image de référence.
- [ ] Faire tourner la suite `metal-suite` directement contre le service, sans macOS invité.

**Terminé quand** la suite Metal passe sur la 9070 XT avec la même tolérance que sur NVIDIA.

### J5 : Rejouer une vraie session macOS

- [ ] Ajouter à upstream un enregistreur du trafic `wire` (requêtes et commandes encodées) entre macOS et le service. C'est utile aussi pour NVIDIA : candidat à une PR upstream.
- [ ] Rejoueur sous Linux : relit un enregistrement sur le service RADV, et compare les images présentées et les résultats de calcul.
- [ ] Obtenir, via le Discord, un enregistrement complet sur une machine NVIDIA : démarrage, assistant de configuration, bureau, Finder, Safari.
- [ ] Rejouer jusqu'à ce que l'écart soit nul ou dans les tolérances de Metal.

**Terminé quand** l'assistant de configuration et le bureau sont rejoués sans écart notable sur la 9070 XT. C'est la preuve que le premier démarrage sera accéléré.

### J6 : Portage d'amdgpu dans Stela

Le plus gros chantier. Mise au point dans la VM avec la carte prêtée (voir « Développement en VM »), puis vérification sur la machine réelle avec le journal de démarrage sur la clé USB.

- [ ] Ajouter `drivers/gpu/drm/amd/` (amdgpu, display/DC, pm/swsmu, include) à `third_party/linux.sparse`, sans amdkfd.
- [ ] Compiler amdgpu sur `stela-linux` et écrire les patches de compatibilité nécessaires (`third_party/patches/amdgpu/`).
- [ ] Créer `amdgpu.rs` (enregistrement du pilote au démarrage) et `amd.rs` (PCI, MMIO, DMA, pages), sur le modèle de `nouveau.rs` et `nvidia.rs`.
- [ ] Firmwares : ajouter au provisionnement de `stela-cli` les fichiers `amdgpu/` de linux-firmware pour la Navi 48 (gc_12_0_1, psp_14_0_3, smu_14_0_3, sdma_7_0_1, dcn_4_0_1, noms à confirmer).
- [ ] Démarrage côté Stela : réclamer les cartes AMD dans `engine/gpu.rs` et `engine/start.rs`, et donner les messages d'état AMD dans `gpu_status.rs`.
- [ ] Étapes d'initialisation à valider dans le journal : IP discovery, PSP, SMU, file graphique MES, SDMA, puis le premier paquet de commandes exécuté.
- [ ] Affichage par DC/DCN via DRM, que `metal/kms.rs` utilise sans modification.

**Terminé quand** amdgpu démarre la 9070 XT dans Stela et que la console passe sur l'écran géré par DCN.

### J7 : RADV dans l'image de démarrage et premier démarrage de macOS

- [ ] `xtask/src/gpu/radv.rs`, sur le modèle de `nvk.rs` : compiler Mesa avec `-Dvulkan-drivers=amd` pour l'image UEFI.
- [ ] Couche de liaison entre RADV et amdgpu dans Stela : adapter libdrm_amdgpu, ou écrire une liaison propre comme celle de nouveau.
- [ ] Brancher `metal::amd::open` au démarrage.
- [ ] Premier démarrage de macOS 27 en VM, avec la 9070 XT prêtée, puis l'assistant de configuration.
- [ ] Le même démarrage sur la machine réelle.

**Terminé quand** l'assistant de configuration est terminé et le bureau est fluide sur la 9070 XT, en VM puis sur la machine réelle.

### J8 : Stabilisation et extension

- [ ] Gestion de l'énergie : ventilateurs, fréquences, mise en veille.
- [ ] Affichage direct depuis la mémoire système, sans copie en mémoire vidéo.
- [ ] Performances : cache de pipelines, ordonnancement des soumissions.
- [ ] RX 9060 (Navi 44), puis éventuellement RDNA 3.
- [ ] Documentation utilisateur et publication d'une release du fork.

## Risques

| Risque | Effet | Parade |
| --- | --- | --- |
| amdgpu est bien plus gros que nouveau, et la couche Linux de Stela ne couvre que ce qu'il fallait pour nouveau | J6 long, beaucoup de patches | Avancer bloc par bloc (PSP, puis GFX, puis DC), en s'appuyant sur le journal de démarrage |
| Upstream change très vite (des centaines de commits par jour) | Conflits fréquents | Isoler le code AMD, synchronisation automatique quotidienne |
| Aucun enregistrement NVIDIA disponible pour J5 | Pas de preuve avant le premier démarrage | J4 et la suite Metal couvrent déjà l'essentiel ; demander tôt sur le Discord |
| Firmware PSP signé qui refuse de se charger hors Linux | J6 bloqué | Reproduire exactement la séquence d'amdgpu ; vérifier les firmwares contre leurs empreintes |
| Pas de carte NVIDIA pour comparer sur la même machine | Débogage à l'aveugle en J6-J7 | Développement en VM avec l'écran VGA émulé et les journaux sous Linux |
| Le bug de réinitialisation des Radeon en VFIO | Hôte figé ou redémarré pendant J6-J7 | Laisser amdgpu prendre la carte au démarrage ; tester les méthodes de réinitialisation ; accepter quelques redémarrages |

## Questions ouvertes

- RADV sur la couche de liaison existante (libdrm), ou une liaison propre à Stela ?
- Ce fork reste-t-il indépendant à long terme, ou propose-t-on à terme une fusion de la partie AMD dans upstream ?
- Prendre en charge RDNA 3 (RX 7000) une fois RDNA 4 stable ?

## Références

- Les versions IP de chaque produit AMD : <https://docs.kernel.org/gpu/amdgpu/amd-hardware-list-info.html>
- Le pilote amdgpu de Linux : `drivers/gpu/drm/amd/`
- RADV et ACO dans Mesa : `src/amd/vulkan/`, `src/amd/compiler/`
- Upstream : <https://github.com/tylertms/Stela>
