# Compatibilite Picasa

Les prochains developpements lies a la compatibilite Picasa doivent suivre la regle TDD de `docs/development.md`.

## Decision
MyCasa vise une compatibilite pragmatique avec Picasa via les fichiers sidecar `.picasa.ini`, pas via la base interne binaire de Picasa.

La base Picasa d'origine est locale au profil utilisateur et contient des fichiers optimises/proprietaires (`.pmp`, `.db`, caches de miniatures). Elle n'est pas un format d'echange stable. En revanche, Picasa maintient aussi un fichier cache `.picasa.ini` dans chaque dossier photo pour sauvegarder des informations par image. C'est le format compatible que MyCasa doit lire et, plus tard, ecrire avec prudence.

Sources de comportement utilisees pour cette decision :
- Picasa conserve un index local et des miniatures pour afficher les bibliotheques rapidement.
- Picasa garde les originaux a leur emplacement et ne les copie pas dans sa base.
- Picasa maintient des fichiers `.picasa.ini` caches dans les dossiers photo comme copie sidecar des edits et informations par fichier.

## Strategie MyCasa
- Les originaux restent a leur emplacement, comme dans Picasa.
- SQLite reste l'index local rapide de MyCasa.
- Les miniatures MyCasa sont cachees localement dans le profil utilisateur et ne cherchent pas a reproduire les fichiers de cache binaires Picasa.
- `.picasa.ini` est lu pendant l'indexation pour importer les infos connues par Picasa.
- L'ecriture de `.picasa.ini` sera ajoutee seulement pour des actions utilisateur explicites, afin d'eviter de modifier silencieusement les dossiers photo ou NAS.

## Champs Importes En Premier
- Section `[nom-du-fichier]` correspondant au fichier image.
- `caption` vers `photos.picasa_caption`.
- `keywords` vers `photos.picasa_keywords`.
- `star` ou `favorite` vers `photos.picasa_starred`.
- `filters` vers `photos.picasa_filters` pour conserver la trace des edits Picasa sans les interpreter.
- `Contacts2` + `faces=rect64(...),contact_id` vers `photo_faces`.

## Analyse De La Base Locale Trouvee
Une installation Picasa PlayOnLinux a ete trouvee ici :

`/home/kassabji/.PlayOnLinux/wineprefix/Picasa/drive_c/users/kassabji/Local Settings/Application Data/Google/Picasa2/`

Fichiers observes :
- `Picasa2Albums/watchedfolders.txt` pointe vers `Z:\mnt\nas_Media\Photos_sorted\`.
- `contacts/contacts.xml` contient les personnes Picasa avec `id`, `name`, `modified_time`.
- `db3/` pese environ 6.3 Go et contient les fichiers Picasa proprietaires.
- `db3/albumdata_filename.pmp` contient les chemins de dossiers/albums en clair.
- `db3/imagedata_caption.pmp`, `imagedata_tags.pmp`, `imagedata_facerect.pmp`, `imagedata_crop64.pmp`, etc. contiennent des donnees utiles, mais dans un stockage binaire colonne/index proprietaire.
- `db3/thumbs_0.db`, `thumbs2_0.db`, `bigthumbs_0.db`, `previews_*.db` sont les gros caches de miniatures/previews Picasa.

Decision actuelle : importer automatiquement les formats lisibles et stables (`watchedfolders.txt`, `contacts.xml`, `.picasa.ini`) avant de tenter une lecture directe des `.pmp/.db`. Les `.pmp` seront documentes puis traites champ par champ quand leur association d'index image sera comprise.

## Implementation Actuelle
- Bouton `Charger dossiers Picasa` : lit `watchedfolders.txt` depuis le profil Picasa PlayOnLinux detecte et traduit les chemins `Z:\...` vers `/...`.
- Import `.picasa.ini` pendant l'indexation : captions, keywords, favoris, filters et visages.
- Table `photo_faces` : stocke `rect64`, `contact_id`, `contact_name` pour chaque visage importe.
- Le viewer affiche le nombre de visages Picasa associes a la photo.
- Tests unitaires existants : parsing `.picasa.ini`, conversion de chemins Wine, migrations SQLite, upsert photo/faces, extensions image supportees.

## Limites Assumee
- Pas de compatibilite binaire avec la base Picasa (`.pmp/.db`).
- Pas d'application des edits `filters` dans le rendu V1.
- Pas d'ecriture automatique de `.picasa.ini` tant que les operations albums/tags/edits MyCasa ne sont pas finalisees.
