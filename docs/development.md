# Developpement

## Regle Obligatoire: TDD
Tout prochain developpement MyCasa doit etre fait en TDD.

Concretement :
- Ecrire ou mettre a jour un test qui echoue avant de modifier le comportement applicatif.
- Implementer le minimum de code pour faire passer ce test.
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
