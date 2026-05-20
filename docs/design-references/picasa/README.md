# References Visuelles Picasa

Ces captures servent de references internes pour rapprocher MyCasa de l'esthetique Picasa 2/3 sans copier les assets proprietaires.

## Fichiers
- `01-library-grid-main.png` : phototheque classique, sidebar dossiers, grille centrale, tray bas.
- `02-library-view-menu.png` : menu Affichage et style Windows classique.
- `03-options-file-types.png` : dialogue Options, onglets et boutons Windows.
- `04-edit-view-effects.png` : vue edition, panneau gauche et grande image centrale.
- `05-folder-manager.png` : gestionnaire de dossiers et liste de dossiers surveilles.
- `06-library-wide-grid.png` : phototheque large avec volet personnes.
- `07-edit-view-wide.png` : vue edition large, toolbar retouche et panneau personnes.
- `08-dialog-reference.png` : dialogue secondaire de reference.
- `09-dialog-folder-monitoring.png` : dialogue de surveillance de dossiers.
- `10-library-and-edit-comparison.png` : comparaison phototheque et edition sur grand ecran.
- `11-library-flat-chronological-reference.png` : phototheque Picasa 3 en vue plate/rétrochronologique, dossiers groupes par annees dans la sidebar et grille principale dense.

## Decisions UI A Reprendre
- Barre de menu fine en haut, puis toolbar grise avec boutons texte + petites icones.
- Sidebar claire a gauche, sections Albums/Dossiers/Personnes repliables.
- Grille centrale type lightbox, fond blanc casse, miniatures avec ombre legere.
- Vue plate/rétrochronologique : separateurs d'annee dans la sidebar, contenu central regroupe par dossier/date, grille dense avec marges regulieres.
- Barre de filtres Picasa 3 : petite bande centrale sous la toolbar avec icones de filtre, curseur et recherche longue a droite.
- Bande bleue de statut au-dessus du tray bas.
- Tray bas pour selection et actions rapides : Album Web, E-mail, Imprimer, Exporter.
- Vue edition/viewer avec grand canevas image et panneau outil lateral.

## Etat MyCasa
- Chrome phototheque Picasa-like : menu, toolbar, sidebar, lightbox, tray bas.
- Volet Personnes droit minimal pour retrouver la composition Picasa 3 large.
- Bande bleue de statut au-dessus du tray bas.
- Viewer edition : panneau outils gauche, filmstrip haut et canevas gris central.
- Viewer clair : fond gris Picasa, panneau outils pale et image centrale plus grande.
- Viewer integre : la vue edition remplace la phototheque au lieu de flotter au-dessus.
- Phototheque : section Chronologie dans la sidebar et bande Filtres/Search inspiree Picasa 3.
- Phototheque : Chronologie de sidebar en hierarchie annee > mois, pour naviguer comme les regroupements temporels Picasa.
- Phototheque : header de mois sticky dans la grille chronologique pour garder le contexte pendant les transitions entre mois.
- Phototheque : suivi du mois visible dans la sidebar Chronologie avec selection automatique annee/mois pendant le scroll.
- Phototheque : clic sur un mois de la sidebar Chronologie positionne la grille principale sur le mois correspondant.
- Phototheque : actions de collection sous le titre et tray bas a boutons fixes plus proche Picasa 3.
- Phototheque : grille recentree et tuiles plus rectangulaires avec ombre legere, pour se rapprocher de la densite de `11-library-flat-chronological-reference.png`.
- Phototheque : clic simple sur une vignette selectionne la photo, double-clic ouvre le viewer, et la bande bleue basse affiche les details de la photo selectionnee.
- Phototheque : les cadres de vignettes suivent le ratio reel de l'image affichee, notamment un cadre vertical serre pour les photos portrait.
- Phototheque : la touche Entree ouvre en viewer la photo selectionnee, les ombres de vignettes sont peintes autour du cadre reel de l'image, et la selection utilise un double contour serre plutot qu'un fond de cellule bleu.
- Phototheque : la navigation clavier aux fleches maintient la vignette selectionnee visible en ajustant le scroll principal.
- Phototheque : la bande bleue basse suit davantage Picasa en affichant nom, date, dimensions et taille de la photo selectionnee, avec les compteurs techniques seulement au survol.
