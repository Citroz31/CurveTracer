# CurveTracer

Outil de tracé de courbes **rapide et interactif** à partir de fichiers de mesures
CSV / Excel, écrit en **Rust** (interface [egui](https://github.com/emilk/egui) +
[egui_plot](https://github.com/emilk/egui_plot)).

Pensé pour les fichiers de test RF (S-paramètres, bruit, puissance, IMD…) :
on charge les fichiers, on ajoute les graphes un par un (X, Y, filtres, séries),
on règle la mise en page, puis on exporte le tout dans un PowerPoint.

📘 **[Guide utilisateur (PDF)](docs/CurveTracer_Guide_utilisateur.pdf)** · [version en ligne](docs/GUIDE_UTILISATEUR.md) · [⬇ Télécharger CurveTracer.exe](https://github.com/Citroz31/CurveTracer/releases/latest)

![Page Plots](docs/screenshots/plots.png)

## Fonctionnalités

### Espaces de travail
- **Écran d'accueil** : créer un nouveau workspace (nom + dossier) ou en ouvrir
  un, avec la **liste des workspaces récents**.
- Menu **📁** de la barre du haut : nouveau, ouvrir, récents, enregistrer,
  retour à l'accueil — avec demande d'enregistrement des modifications.

![Écran d'accueil](docs/screenshots/start.png)

### Données (page *File Select*)
- CSV / TXT / TSV (délimiteur `,` `;` tabulation `|` détecté automatiquement) et
  Excel (`xlsx`, `xlsm`, `xlsb`, `xls`, `ods`).
- **Détection automatique de la ligne d'en-tête** : les lignes de métadonnées
  écrites au-dessus des noms de colonnes (Test Request ID, Part Number…) sont
  ignorées et conservées dans « File information ».
- Virgule décimale (`1,5`) gérée automatiquement, valeurs `N/A` acceptées,
  zéros de tête conservés (SN `07102601`).
- Options forçables par fichier (délimiteur, ligne d'en-tête, virgule décimale,
  feuille Excel) + rechargement.
- Plusieurs sources de données simultanées, glisser-déposer des fichiers sur la fenêtre.

### De-embedding des accès (`.s2p`)
- Bouton **🔌 De-embedding** de chaque fichier : fichiers Touchstone deux-ports
  d'**entrée** (instrument → DUT) et de **sortie** (DUT → instrument), option
  *Flip ports* (formats MA / DB / RI, Hz…GHz, Touchstone v1 et v2).
- **Vectoriel** (matrices de transfert T) quand les données contiennent module
  et phase des quatre paramètres (`S11db`, `S11deg` … `S22deg`) et que les
  fichiers ont des phases, **scalaire** (soustraction des pertes en dB) sinon.
- Interpolation en fréquence (dB et phase dépliée), unité détectée.
- Colonnes corrigées en place (les graphes se mettent à jour), copie des
  valeurs brutes (`S21db raw`) ; correction optionnelle d'autres colonnes
  (puissances d'entrée / sortie, gain, NF).

![De-embedding](docs/screenshots/deembedding.png)

### Performances
- Chargement **multi-cœur** : le fichier est découpé en blocs analysés en
  parallèle ; chaque colonne est encodée en dictionnaire (une recherche de hachage
  par cellule, conversion numérique une seule fois par valeur unique).
  Ordre de grandeur : 1 000 000 de lignes × 29 colonnes de mesures (277 Mo) chargées en < 1 s,
  ~60 Mo en mémoire.
- Calcul des courbes (filtres, séries, agrégations) en tâche de fond : l'interface
  ne se fige jamais (4 ms pour filtrer/grouper 500 000 lignes).
- Affichage des très grosses courbes par décimation min/max adaptée au zoom
  (visuellement sans perte).

### Graphes (page *Plots*)
Onglet **Data**
- **Select Data** : source de données, colonnes (triables par nom ou nombre de
  valeurs uniques, recherche), à glisser-déposer ou via les boutons `X` `Y` `F` `S` :
  - **X-Data** (limite 1) avec **modificateur** : `÷1e9 (Hz → GHz)`, `÷1e6`,
    `÷1e3`, `×1e3 (A → mA)`, `×1e6`, `×1e9`, facteur personnalisé —
    le libellé d'axe s'adapte (`Freq (GHz)`, `Courant [mA]`) ;
  - **Y-Data** : une ou plusieurs colonnes (même modificateur possible) ;
  - **Filter** : sélection de valeurs (cases à cocher, *All / None / Invert / only*,
    recherche) ou plage numérique `[min, max]` ;
  - **Series** : une courbe par valeur (SN, température, part number…),
    plusieurs colonnes combinables.
- **Process Data** : moyenne, médiane, min, max, somme, RMS, écart-type,
  étendue, comptage — par X, en regroupant les séries choisies
  (ex. moyenne sur les SN → une courbe par température), avec ou sans les courbes
  d'origine.
- **Smoothing** : lissage des courbes — moyenne glissante, gaussien,
  Savitzky–Golay, médiane, exponentiel sans déphasage, moyenne sur une largeur
  en X — avec fenêtre, ordre et nombre de passes réglables.
- **View Data** : table (virtualisée) des lignes utilisées par le graphe.

Onglet **Layout** : titre (affiché ou non), libellés X / Y, police / taille / couleur
des titres, des libellés et des graduations, plages d'affichage (auto ou manuelles,
bouton « Use current zoom »), pas des graduations X / Y, nombre de décimales,
grille, légende (position, titre, police, fond, bordure), couleurs.
Bouton « Apply this layout to all plots ».

Onglet **Plot Options** : tri par X, nouvelle ligne quand X repart en arrière
(balayages successifs), lecture des valeurs au survol, réticule, curseurs, export.

Onglet **Trace Options** : palette, épaisseur (2 px par défaut), style de ligne,
marqueurs, style **Custom** (motif de tirets modifiable), **styles des
températures** (quand la série est une température : −40 °C bleu tiret-point,
25 °C vert plein, 85 °C rouge tirets), puis
pour chaque courbe : visibilité, couleur, épaisseur, style, marqueur,
**libellé de légende personnalisé**.

Interaction
- **Molette : graphe suivant / précédent** (graphes empilés, liste avec
  miniatures à gauche, Page Up / Page Down).
- Ctrl + molette : zoom · glisser : déplacement · clic droit + glisser : zoom
  rectangle · glisser sur un axe : zoom de cet axe · double-clic : échelle d'origine.
- Outils : *Pan* / *Zoom area*, zoom + / −, *Fit Y*, *Previous view*,
  *Original scale*, *Axes & title…*.
- **Clic sur un axe** : le renommer, régler sa plage et son pas ; **clic sur le
  titre** : l'éditer.
- Survol : valeurs x / y de la courbe la plus proche.
- **Clic sur une courbe : pose un curseur** affichant x / y (re-clic pour l'enlever).
- **La légende se déplace à la souris** ; déposée contre un bord du graphe, elle
  passe **à l'extérieur du cadre et le graphe se redimensionne** automatiquement.
  Un clic sur une entrée masque / affiche la courbe. Au-delà d'une certaine hauteur,
  la légende se répartit sur plusieurs colonnes.

![Process Data](docs/screenshots/process.png)

### Fonctions utilisateur (page *Functions*)
Colonnes calculées par formule, disponibles dans toutes les sources :
`[Freq] / 1e9`, `[S21db] - [Thru Loss]`, `db20(...)`, `lin10(...)`, `dbm2w(...)`,
`sqrt`, `log10`, `abs`, `min`, `max`, `^`… Les noms de colonnes s'écrivent entre
crochets (`[VDD Supply Current [A]]`) ou directement s'ils sont simples (`S21db`).

### Export des graphes (page *Export Plots*)
- Miniatures de tous les graphes, sélection, titre de chaque diapositive.
- **Download PowerPoint** : une diapositive 16:9 par graphe (titre + image haute
  résolution), format *Plot only* ou *Plot + Settings* (source, X, Y, séries,
  filtres, traitement).
- Graphes intégrés en **SVG vectoriel** (avec copie PNG de secours) dans le PowerPoint.
- **Download Images** (PNG haute résolution ou **SVG** vectoriel) et **Download Data** (CSV des courbes).
- Boutons **PNG** / **SVG** de la page *Plots* pour enregistrer le graphe affiché.
- Option « Use the current zoom of each plot » pour exporter exactement la vue affichée.

![Export](docs/screenshots/export.png)
![Diapositive](docs/screenshots/slide-1.png)

### Export des données (page *Export Data*)
- Données **après de-embedding** (ou brutes) en **CSV** (virgule, point-virgule, tabulation, virgule décimale)
  et / ou **Touchstone `.s2p`** (DB / MA / RI, Hz … GHz, impédance de référence) — un `.s2p` par courbe.
- Choix des colonnes, de la **bande de fréquence** et de **critères** (SN, température, gamme de puissance…).
- **Rangement** en sous-dossiers / noms de fichiers par valeur des colonnes choisies (ex. un dossier par SN,
  un fichier par température) ; courbes détectées automatiquement (SN, température, état…).
- **Lissage colonne par colonne** (défaut / personnalisé / aucun, toutes les méthodes du lissage des graphes) ;
  courbes lissées sur toute leur plage puis coupées à la bande, phases déroulées.
- Aperçu de l'arborescence et de chaque courbe lissée ; export en tâche de fond (~2 s par million de lignes).

![Export Data](docs/screenshots/export_data.png)

### Espace de travail
`File → Save workspace` / **QuickSave** (`Ctrl+S`) : sauvegarde des sources,
fonctions et réglages de tous les graphes dans un fichier `.ctw` (JSON),
rechargeable par `File → Open workspace`, glisser-déposer ou en argument de
ligne de commande (`CurveTracer mon_projet.ctw`).

### Polices
Les polices Arial, Calibri, Segoe UI, Times New Roman, Courier New, Consolas,
DejaVu… sont proposées si elles sont installées ; `File → Load a font file`
ajoute n'importe quel `.ttf` / `.otf`. Elles sont utilisées à l'écran **et** dans
les exports.

## Installation

### Binaire prêt à l'emploi
Télécharger **`CurveTracer.exe`** depuis la page
[**Releases**](https://github.com/Citroz31/CurveTracer/releases/latest) et le lancer
(aucune installation). Avertissement SmartScreen au premier lancement :
*Informations complémentaires → Exécuter quand même*, ou *clic droit → Propriétés →
Débloquer* ; pour le supprimer définitivement (signature de code), voir
[docs/SIGNATURE_WINDOWS.md](docs/SIGNATURE_WINDOWS.md). Le **[guide utilisateur (PDF)](docs/CurveTracer_Guide_utilisateur.pdf)**
y est aussi joint ([version en ligne](docs/GUIDE_UTILISATEUR.md)).

Chaque push sur `main` compile aussi l'application (onglet **Actions**, artefact
`CurveTracer-windows-x64`) ; pousser un tag `v*` (ex. `v0.1.0`) crée une nouvelle
*release* avec l'exécutable Windows, le guide et les archives Windows / Linux / macOS.

### Compilation depuis les sources
Prérequis : [Rust](https://rustup.rs) ≥ 1.95.

```bash
git clone https://github.com/Citroz31/CurveTracer
cd CurveTracer
cargo run --release
```

Sous Linux, quelques bibliothèques système peuvent être nécessaires :
`sudo apt install libxkbcommon-dev libwayland-dev`.

Des fichiers d'exemple sont fournis : `sample_data/CGY2272_S-Parameters_example.csv`
(données synthétiques, même structure qu'un fichier de S-paramètres : métadonnées
puis en-tête en ligne 5, 7 SN × 3 températures × 2 tensions VDD) et deux accès
`sample_data/fixture_input.s2p` / `fixture_output.s2p` pour essayer le de-embedding.

## Organisation du code

| Dossier | Rôle |
|---|---|
| `src/data/` | chargement CSV (parallèle) / Excel, table en colonnes encodées en dictionnaire, moteur de formules |
| `src/plot/config.rs` | description d'un graphe (sérialisée dans les espaces de travail) |
| `src/plot/compute.rs` | filtres → séries → agrégations → segments de courbes |
| `src/plot/view.rs` | graphe interactif (egui_plot), légende déplaçable, curseurs |
| `src/plot/render.rs` | rendu PNG (tiny-skia + ab_glyph) et SVG pour les exports |
| `src/data/deembed.rs` | lecture Touchstone `.s2p`, de-embedding vectoriel / scalaire |
| `src/plot/geom.rs` | géométrie partagée : placement de la légende, décimation, graduations |
| `src/export/` | écriture PowerPoint (`.pptx`) et ZIP, export des données CSV / Touchstone (`data.rs`) |
| `src/ui/` | pages de l'interface (`start.rs` : écran d'accueil des workspaces) |
| `src/recent.rs` | mémoire des workspaces récents |

Tests : `cargo test`.
