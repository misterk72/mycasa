# MyCasa: Sauvegarde Du Plan Et Démarrage

## Résumé
Créer le projet natif MyCasa dans `/home/kassabji/workspace/mycasa`, sauvegarder le plan dans `plans/`, puis initialiser une application Rust desktop portable Windows/Linux.

## Étapes Initiales
- Créer `/home/kassabji/workspace/mycasa/plans`.
- Sauvegarder le plan projet dans `/home/kassabji/workspace/mycasa/plans/initial-plan.md`.
- Initialiser un projet Rust dans `/home/kassabji/workspace/mycasa`.
- Configurer une app desktop avec `eframe/egui`, rendu `wgpu`, et une fenêtre principale minimale.
- Ajouter la structure de base : catalogue, indexeur, miniatures, UI, viewer.

## Stack Retenue
- Rust pour performance, portabilité et binaire natif.
- `eframe/egui` pour UI desktop native-like rapide.
- `wgpu` pour rendu fluide.
- SQLite pour catalogue local.
- Workers asynchrones pour scan, métadonnées et miniatures.

## Première Implémentation
- Écran principal avec barre latérale dossiers/albums, grille centrale et panneau d’état.
- Base SQLite initialisée au lancement.
- Modèle minimal pour les photos indexées.
- Indexeur de dossier stub prêt à connecter au scan réel.
- Cache de miniatures stub, sans retouche ni ML en V1.

## Tests
- Les prochains développements doivent obligatoirement suivre une approche TDD : test échouant d'abord, implémentation minimale ensuite, puis refactorisation à tests verts.
- `cargo check` doit passer.
- `cargo test` doit passer.
- L’application doit démarrer sur Linux.
- La fenêtre principale doit s’ouvrir sans blocage.
- La structure doit permettre l’ajout incrémental de l’indexation réelle.

## Hypothèses
- `/home/kassabji/workspace/mycasa` reste le dossier racine.
- Le dossier est vide et peut recevoir un nouveau projet.
- La V1 reste limitée à catalogue + viewer.
