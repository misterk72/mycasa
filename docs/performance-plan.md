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
- Chargement catalogue pagine par pas de 500 photos, plafonne a 5000 pour proteger la grille.
- Cache memoire miniatures borne a 800 textures pretes, avec regeneration possible depuis le cache disque.
- Demandes miniatures visibles prioritaires dans la file de chargement.
- Index SQLite explicites sur chargement recent, recherche fichier/dossier et visages par photo.
- Recherche debouncee a 250 ms pour eviter une requete catalogue a chaque frappe.
- Viewer charge une preview depuis le cache miniature avant l image 1600 px, ignore les reponses obsoletes, et adapte sa taille initiale au ratio de la photo.
- Le viewer conserve un cache memoire borne des images 1600 px prechargees, draine les decodages termines meme quand la phototheque est affichee, limite les decodages concurrents, et attend que l'image courante soit chargee avant de lancer les prechargements voisins.
- Le format HEIC/HEIF n'est pas encore pris en charge par le decodeur actuel ; il doit etre ajoute avec un backend natif portable et teste avant activation dans l'indexeur.
