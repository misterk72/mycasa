# Developpement

## Regle Obligatoire: TDD
Tout prochain developpement MyCasa doit etre fait en TDD.

Concretement :
- Red : ecrire ou mettre a jour un test qui echoue avant de modifier le comportement applicatif.
- Green : implementer le minimum de code pour faire passer ce test.
- Refactor : nettoyer la conception, supprimer la duplication et ameliorer les noms/decoupages sans changer le comportement, en gardant les tests au vert.
- Lancer `cargo test` avant de considerer la modification terminee.
- Lancer `cargo check` pour verifier le build courant.
- Pour les changements UI difficiles a tester unitairement, isoler la logique testable dans un module ou une fonction pure, puis tester cette logique avant de brancher l'UI.

Exceptions autorisees :
- Changement purement documentaire.
- Renommage ou formatage mecanique sans changement de comportement.
- Investigation non mutante.

## Zones A Proteger Par Tests
- Parsing `.picasa.ini` et import des visages Picasa.
- Migration SQLite et compatibilite avec les anciens catalogues.
- Upsert de photos et remplacement des metadonnees/faces.
- Conversion des chemins Picasa/Wine vers chemins Linux.
- Indexation de dossiers et filtrage des extensions image.
- Cache disque de miniatures.

## Backlog Priorise
1. Faisabilite fluidite : virtualiser la grille, mesurer cache/generation de miniatures et afficher un compteur FPS approximatif.
2. Miniatures rapides : valider le cache disque par tests unitaires et eviter de regenerer les miniatures existantes.
3. Viewer fluide : charger d'abord la preview cachee, precharger les images 1600 px autour de la photo courante, mesurer les hits/misses du cache viewer, puis eviter les decodages repetes depuis le NAS.
4. Formats modernes : ajouter HEIC/HEIF apres choix d'un decodeur portable Windows/Linux et couvrir indexation, miniatures et viewer par tests.
5. Esthetique Picasa-like : tuiles plus propres, toolbar compacte, et etats visuels de chargement/selection/hover.
6. Vues phototheque : conserver la vue dossiers/arborescence et enrichir la vue retrochronologique a plat avec des en-tetes de periode plus proches de Picasa.
7. Import Picasa robuste : parser `contacts.xml`, relier les contacts aux faces `.picasa.ini`, puis exposer les noms dans le viewer.
8. Catalogue exploitable : ajouter filtres favoris/visages/mots-cles et recherche par metadata Picasa.

Chaque item doit etre implemente par petits increments TDD avec un test rouge avant le code de production.
