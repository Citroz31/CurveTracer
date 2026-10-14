## Télécharger

| Fichier | Contenu |
|---|---|
| **CurveTracer.exe** | l'application Windows (64 bits), à lancer directement — aucune installation |
| **CurveTracer_Guide_utilisateur.pdf** | le guide utilisateur illustré |
| CurveTracer-windows-x64.zip | exécutable + guide + fichier d'exemple |
| CurveTracer-linux-x64.zip / CurveTracer-macos.zip | versions Linux et macOS |

> Au premier lancement, Windows SmartScreen peut afficher « Windows a protégé votre ordinateur » (exécutable non signé) :
> **Informations complémentaires → Exécuter quand même**.

## Nouveautés de la version 0.3.1 — chargement des gros fichiers

- **Chargement CSV jusqu'à 14× plus rapide** sur les vrais fichiers de mesure (1 M lignes, 277 Mo : 13 s → < 1 s) :
  les colonnes de mesures (valeurs presque toutes différentes) passent automatiquement en stockage numérique direct au lieu d'un dictionnaire.
- Conversion des nombres plus rapide (exacte), découpage des lignes vectorisé, valeur répétée d'une ligne à l'autre sans hachage.
- Fichiers **entre guillemets** chargés en parallèle (2× plus rapide).
- Colonnes texte à valeurs uniques (numéros de série, horodatages) : 9 s → 1 s (meilleur hachage, tri naturel sans allocation et calculé à la demande pour les très grandes listes).

## Nouveautés de la version 0.3.0

- **Écran d'accueil des espaces de travail** : créer un nouveau workspace (nom + dossier) ou en ouvrir un, avec la **liste des workspaces récents** (emplacement, dernière utilisation).
  Menu **📁** dans la barre du haut pour créer / ouvrir / changer de workspace à tout moment, avec demande d'enregistrement des modifications.
- **Navigation à la molette** : les graphes sont empilés, la **molette** passe au graphe suivant / précédent (Page Up / Page Down aussi) ; liste des graphes avec miniatures à gauche (réordonner, dupliquer, supprimer). Le zoom passe à **Ctrl + molette**.
- **Clic sur un axe** pour le renommer et régler sa plage, son pas et la grille ; **clic sur le titre** pour l'éditer.
- **Outils de zoom** : *Pan* / *Zoom area* (rectangle au clic gauche), zoom + / −, *Fit Y*, *Previous view*, *Original scale*, *Axes & title…*.
- **Styles des températures** : −40 °C **bleu tiret-point** (nouveau style *Custom*, motif modifiable), 25 °C **vert plein**, 85 °C **rouge tirets** — aussi dans les exports PNG / SVG / PowerPoint.
- Titre de l'application lisible sur la barre bleue (nouveau logo).
- Graduations automatiques 1 / 2 / 5 : une plage étroite est toujours graduée.

## Nouveautés de la version 0.2.0

- **De-embedding** des accès d'entrée / sortie à partir de fichiers Touchstone `.s2p` (bouton 🔌 *De-embedding* de chaque fichier) :
  **vectoriel** (matrices T) quand les colonnes `S11db/S11deg … S22db/S22deg` et les phases des fichiers sont disponibles, **scalaire** (pertes en dB) sinon ;
  interpolation en fréquence, *Flip ports*, valeurs brutes conservées (`S21db raw`), correction optionnelle des puissances / gain / NF.
- **Couleurs des températures** par défaut : quand la série est une température (`Set_TempC`…), −40 °C en **bleu**, 25 °C en **vert**, 85 °C en **rouge**.
- Épaisseur des courbes par défaut : **2 px**.
- **Export SVG** (vectoriel) : bouton *SVG* de la page Plots, *Download Images* au format PNG ou SVG, et graphes SVG (avec copie PNG) dans le PowerPoint.
- Graduations en notation scientifique plus lisibles (`1.25e10`).

## Contenu

- Chargement rapide (multi-cœur) de fichiers CSV / TXT / Excel, détection automatique du délimiteur et de la ligne d'en-tête.
- Graphes construits par X (avec modificateur Hz → GHz…), Y, filtres et séries ; moyenne / min / max / somme / RMS…
- Zoom, lecture des valeurs, curseurs par clic, légende déplaçable à la souris (y compris hors du cadre).
- Mise en page complète (titres, polices, graduations, plages, légende), styles par courbe, fonctions utilisateur.
- Export PowerPoint, images PNG / SVG et CSV ; espaces de travail `.ctw`.
