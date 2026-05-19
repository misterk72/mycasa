# Plan Performance Et Fluidite

## Objectif
Valider rapidement que MyCasa peut afficher un dossier photo NAS avec une experience fluide et visuellement credible avant d'approfondir l'import Picasa avance.

## Cibles De Faisabilite
- Demarrage sans freeze avec un catalogue existant.
- Grille scrollable sans rendre les lignes hors ecran.
- Miniatures chargees progressivement, avec cache disque local.
- Aucun decodage image lourd sur le thread UI.
- Statut visible pour diagnostiquer cache, generation, files d'attente et FPS approximatif.

## Mesures Affichees
- `cache` : miniatures lues depuis le cache disque.
- `gen` : miniatures generees depuis l'original.
- `fail` : miniatures impossibles a charger.
- `evict` : textures miniatures retirees de la memoire GPU.
- `pending` : miniatures en attente de worker.
- `active` : workers miniatures actifs.
- `ready` : textures miniatures chargees en memoire.
- `scans` : indexations actives.
- `dbq` : photos en attente d'ecriture catalogue.
- `fps` : compteur approximatif de frames par seconde.
- `viewer source` : origine de l'image 1600 px courante dans le viewer (`memoire`, `cache`, `original`, `erreur`, `attente`).
- `viewer mem/disk/orig/err/att` : hits memoire viewer, hits cache disque 1600 px, decodages originaux, erreurs et decodages en attente.

## Profilage Decodage Viewer
Commande locale :

```bash
cargo build
target/debug/mycasa --profile-decode --limit 12 /mnt/nas_Media/Photos_sorted/2026/04
```

Colonnes importantes :
- `decode_ms` : lecture/decompression du format original.
- `resize_ms` : reduction vers la preview viewer 1600 px.
- `rgba_ms` : conversion vers le buffer RGBA envoye a egui.
- `display_ready_ms` : cout avant affichage possible (`decode + resize + rgba`).
- `png_encode_ms` : cout d'ecriture du cache disque 1600 px, maintenant decale apres l'envoi au viewer.
- `png_decode_ms` : cout estime de relecture du cache viewer 1600 px.

Resultat sur un echantillon NAS 2026/04 :
- En debug non optimise, le pipeline image etait trop lent pour juger la fluidite.
- Avec optimisation dev ciblee sur les crates image/JPEG/PNG, `target/debug/mycasa` descend a environ 491 ms par image pour le premier passage complet, dont environ 281 ms avant affichage possible.
- Le cache PNG 1600 px se relit autour de 31 ms sur l'echantillon, donc les retours sur une photo deja mise en cache doivent etre nettement plus rapides que le premier decode original.
- HEIC/HEIF est maintenant branche via `libheif-rs` et valide sur un echantillon reel Linux/NAS.
- Mesure du 2026-05-19 sur 12 images NAS 2026/04 : `avg_display_ready_ms=296.3`, `avg_total_ms=554.5`, `avg_decode_ms=188.2`, `avg_resize_ms=105.6`, `avg_png_decode_ms=29.2`.
- Le HEIC teste (`20260405_102402.heic`) affiche `display_ready_ms=443.3` et `png_decode_ms=24.2`, ce qui confirme que le premier decode HEIC reste cher mais que le cache 1600 px annule presque tout ce cout aux retours suivants.

Decision technique provisoire :
- Le rendu GPU `wgpu` accelere l'affichage et l'upload texture, mais pas la decompression JPEG/HEIC elle-meme.
- Le gain court terme le plus fiable est CPU/SIMD + cache local : build dev optimise pour les crates de decode, affichage avant ecriture du cache, prechargement autour de la photo courante et cache 1600 px persistant.
- L'acceleration materielle JPEG via GPU n'est pas une cible V1 portable simple Windows/Linux. A etudier plus tard uniquement si le pipeline CPU/SIMD + cache ne suffit pas.
- Pour HEIC, la piste portable retenue est `libheif-rs`/libheif. Le point a surveiller devient le packaging Windows/Linux de la bibliotheque native.

## Priorites D'Implementation
1. Virtualiser la grille avec `show_rows`.
2. Tester et exposer les metriques de miniatures.
3. Ameliorer le rendu des tuiles et de la barre d'outils.
4. Viewer rapide : cache memoire des previews viewer, prechargement autour de la photo courante, puis mesure des hits/misses.
5. Support HEIC/HEIF : choisir un decodeur portable Windows/Linux avant indexation et affichage complets.

## Regle TDD
Chaque changement de logique doit commencer par un test rouge, puis passer au vert, puis etre refactorise si necessaire.

## Etat Courant
- Grille virtualisee avec calcul de colonnes, lignes et plages d'items couvert par tests unitaires.
- Cache disque de miniatures teste : invalidation par taille/mtime, generation initiale, relecture cache sans original disponible, echec explicite si cache et original absents.
- Concurrence miniatures limitee a 3 workers pour reduire la pression sur un NAS.
- Viewer navigable au clavier avec fleches gauche/droite, prechargement des miniatures voisines et prechargement memoire des images viewer dans un rayon de 3 photos autour de la courante.
- Libelles de tuiles tronques au milieu pour eviter les debordements visuels.
- Ecritures catalogue regroupees en transactions par lot pour reduire les pauses pendant l'indexation.
- File evenements indexation bornee a 512 messages pour appliquer une pression retour au scan.
- Scans du meme dossier dedupliques pendant qu'une indexation est deja active.
- Chargement catalogue progressif par pas de 500 photos, declenche automatiquement pres du bas du scroll, sans bouton ni plafond utilisateur visible.
- Fenetre memoire catalogue bornee : apres 3000 photos chargees, MyCasa conserve environ 2000 photos autour de la progression courante et ajuste le scroll apres eviction.
- Rechargement automatique des pages precedentes quand on remonte pres du haut d'une fenetre evincee.
- Sidebar Chronologie alimentee par les mois distincts du catalogue complet, independamment de la fenetre memoire chargee dans la grille.
- Cache memoire miniatures borne a 800 textures pretes, avec regeneration possible depuis le cache disque.
- Demandes miniatures visibles prioritaires dans la file de chargement.
- Vue retrochronologique a plat avec tri par date capturee, date de modification en repli, et en-tetes mensuels virtualises.
- Index SQLite explicites sur chargement recent, recherche fichier/dossier et visages par photo.
- Recherche debouncee a 250 ms pour eviter une requete catalogue a chaque frappe.
- Viewer charge une preview depuis le cache miniature avant l image 1600 px, ignore les reponses obsoletes, et adapte sa taille initiale au ratio de la photo.
- Le viewer conserve un cache memoire borne des images 1600 px prechargees, draine les decodages termines meme quand la phototheque est affichee, limite les decodages concurrents, et attend que l'image courante soit chargee avant de lancer les prechargements voisins.
- Le viewer persiste aussi les images 1600 px dans un cache disque local pour eviter de relire et redecompiler les originaux NAS apres le premier affichage.
- Le viewer envoie maintenant l'image decodee a l'UI avant d'ecrire le cache PNG 1600 px, afin que l'encodage cache ne bloque plus le premier affichage.
- Un mode CLI `--profile-decode` mesure le pipeline image sur de vrais dossiers sans lancer l'interface.
- Le format HEIC/HEIF est accepte par l'indexeur et decode via `libheif-rs` en integration `image`; il reste a mesurer sur plus d'echantillons et a documenter le packaging natif.
- Mesure HEIC initiale : `/mnt/nas_Media/Photos_sorted/2026/04/20260405_102402.heic` decode en environ 388 ms avant affichage possible dans `target/debug/mycasa` avec les crates image optimisees.
