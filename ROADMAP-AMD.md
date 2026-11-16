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

## Où on en est (16 novembre 2026)

Fait depuis Windows, sans la carte :

- Fork `getmowbot/Stela` : les patchs J0 et J2, `vfio-gpu.sh` et cette feuille de route sont dans `main`. Le remote `upstream` pointe sur `tylertms/Stela`, avec `git rerere` activé dans le clone Windows. À refaire dans chaque nouveau clone.
- `amd-ci.yml` passe sur GitHub : rustfmt, clippy et tests de `stela-driver-gpu`, `stela-boot-core` et `stela-amd-host`. Le code AMD compile, mais n'a encore jamais tourné sur la carte.
- `amd-sync-upstream.yml` tourne chaque jour à 4 h 17 UTC. GitHub Actions a le droit d'ouvrir des PR sur le fork. En cas de conflit, la fusion se fait à la main dans `sync/upstream`.
- Les commits du fork portent l'identité `Stela contributor <contributor@example.invalid>`, comme les patchs.

Fait sous Linux (Ubuntu 24.04, noyau 6.14, Mesa 25.2.8) :

- Le clone Linux a son remote `upstream`, `git rerere` activé et l'identité des commits du fork. Rust 1.99.1 est installé par rustup.
- J2 : `stela-amd-host` a tourné sur la 9070 XT, et sa sortie est archivée dans `docs/amd/qualification.md`. Seul manque au départ : `dynamicPrimitiveTopologyUnrestricted`, que RADV ne fournit pas. Ce n'est plus une exigence : sans cette propriété, les pipelines Metal sans classe de topologie compilent une variante pour les points et une pour les lignes. **La carte est maintenant admise** (code de sortie 0), avec l'import dma-buf, Metal3 et Apple7. Limite : ces pipelines ne sont pas proposés aux indirect command buffers.
- J1 : `vfio-gpu.sh status` reconnaît la carte (IOMMU et Resizable BAR actifs, groupes propres). Il n'y a pas d'autre sortie d'affichage : l'écran est branché sur la 9070 XT et le processeur n'a pas de puce graphique active.

- J3 : les compilateurs de Stela sont construits avec la cible AMD et le frontal AIR (LLVM 23). La suite Metal tourne sur un Mac (M3 Pro, macOS 27.2.1) : 14 146 cas réussis, aucun échec. Les shaders de WindowServer, de CoreAnimation et de l'assistant de configuration compilent tous vers NIR pour gfx1201. Sur tout macOS, aucun refus n'est propre à AMD (`docs/amd/shaders.md`).

- J4 : le vrai périphérique Metal de Stela tourne sur la 9070 XT, sous Linux, sur un RADV construit à partir du Mesa de Stela avec le compilateur Metal (`libstela_radv.so`). Le premier calcul et le premier rendu sont vérifiés, et 11 843 cas de la suite Metal (tous les calculs exécutables et 115 rendus) passent à l'identique du Mac (`docs/amd/device.md`). Les corrections d'outillage de J3 sont proposées à upstream ([tylertms/Stela#160](https://github.com/tylertms/Stela/pull/160)).

À faire ensuite, dans cet ordre :

1. Suivre la revue des PR upstream [tylertms/Stela#158](https://github.com/tylertms/Stela/pull/158) (topologie, maintenant vérifiée sur le matériel) et [tylertms/Stela#160](https://github.com/tylertms/Stela/pull/160) (outillage).
2. J4 : étendre l'exécuteur Linux de la suite (`stela-amd-host --suite`) au reste du rendu (profondeur, stencil, sommets, multi-échantillonnage, échantillonneurs), puis aux textures et au reste. Mesurer au passage les alignements des surfaces linéaires. Proposer à upstream la largeur SIMD de 32 et `nir_build_frag_coord`.
3. J3 : ouvrir à upstream des tickets pour les familles de refus du frontal AIR.
4. J1, dernier point : le Ryzen 5 5600X n'a pas de puce graphique, et la 9070 XT est la seule sortie d'affichage. Avant le premier essai en VM, il faut ajouter une petite carte graphique pour l'écran de Linux, ou lancer l'essai par SSH et suivre la VM par VNC. L'utilisateur est maintenant dans le groupe `kvm`.

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
- [x] Proposer le correctif sur la topologie à upstream : [tylertms/Stela#158](https://github.com/tylertms/Stela/pull/158).
- [ ] Remplacer les alignements estimés (256 octets) par les valeurs que RADV renvoie réellement.

**Terminé quand** les fonctions requises sont présentes (19 depuis le correctif sur la topologie), l'import dma-buf aussi, et Metal3 + Apple7 sont atteints sur la vraie carte. C'est atteint depuis le 15 novembre 2026. Restent les manques Metal et les alignements.

### J3 : Compiler les shaders de macOS pour gfx1201 🟡

Sans GPU nécessaire. Mesure et commandes dans `docs/amd/shaders.md`.

- [x] Ajouter la cible AMD gfx1201 à `cargo xtask gpu metal-suite offline` : outil `stela_metal_corpus_amd`, option `--gpu`.
- [x] Extraire les bibliothèques Metal de WindowServer (SkyLight), CoreAnimation (QuartzCore) et de l'assistant de configuration : `metal-suite coverage` les copie depuis un Mac sous macOS 27.2.1.
- [x] Les compiler toutes pour gfx1201 (`xtask/scripts/amd-macos-shaders.py`). Le corpus du démarrage passe en entier, sauf le constructeur BVH du ray tracing et une archive de CoreImage assemblée à l'exécution. Sur tout macOS, aucun refus n'est propre à AMD : gfx1201 et NVIDIA refusent les mêmes 13 319 points d'entrée sur 57 401.
- [ ] Ouvrir à upstream un ticket par famille de refus du frontal AIR commun (`air.signature`, bfloat, accès volatils, budget de métadonnées…).
- [ ] Descendre jusqu'à ACO pour tout le corpus : RADV compile désormais le NIR de Stela en code machine gfx1201 (J4). Reste à le faire pour tous les shaders de macOS et à étendre `coverage` à ce qu'ACO refuse.

**Terminé quand** tous les shaders de ce corpus compilent pour gfx1201, jusqu'au code machine. Le niveau NIR est atteint pour le corpus du démarrage.

### J4 : La couche Metal de Stela tourne sur RADV sous Linux 🟡

Détails et commandes dans `docs/amd/device.md`.

- [x] Construire les compilateurs de Stela avec la cible AMD (`cargo xtask gpu fetch`, Mesa et ses patches, option `compilers=amd`).
- [x] Construire RADV à partir du Mesa de Stela, avec le compilateur Metal lié dans la même bibliothèque : `libstela_radv.so` (option Meson `radv-host`). Correctif RADV `third_party/patches/mesa/9001-radv-take-nir-stages.patch`, pour que RADV prenne le NIR d'un étage comme le fait NVK.
- [x] Mode `--device` de `stela-amd-host` (fonctionnalité Cargo `radv-host`) : le vrai périphérique Metal (`NativeBackend` + `AirCompiler`) s'ouvre sur RADV, et la mémoire partagée passe par `/dev/udmabuf`.
- [x] Premier calcul : 65 536 additions de flottants sur le GPU, résultat vérifié.
- [x] Premier rendu : un triangle dans une texture RGBA8, vérifié par son aire et la couleur de ses coins. Une bande de lignes, dessinée par le même pipeline sans classe de topologie, vérifie la PR #158 sur le matériel.
- [x] Faire tourner la suite `metal-suite` directement contre le périphérique, sans macOS invité (`stela-amd-host --suite`) : 11 727 cas de calcul sur 11 727 identiques au Mac, aucun écart. Corrections en route : `sin` et `cos` dans RADV (patch `9001`), et largeur SIMD de 32 comme sur Apple (`capabilities::simd_width`).
- [x] Premiers cas de rendu de la suite : 115 identiques au Mac, et 2 écarts d'une unité sur les cibles 10 bits (sortie en flottant 16 bits imposée par RB+). Correction en route : le frontal AIR construit la position du fragment dans la forme du pilote (`nir_build_frag_coord`).
- [ ] Étendre l'exécuteur au reste du rendu (profondeur, stencil, sommets, multi-échantillonnage, échantillonneurs), puis aux textures, séquences, blits et API (1 602 cas), et aux bibliothèques compilées à l'exécution (711 cas).
- [ ] Mesurer les vrais alignements des surfaces linéaires sur RADV.

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
