---
title: "CurveTracer — Guide utilisateur"
subtitle: "Version 0.1.0"
---

# 1. Présentation

CurveTracer trace des courbes à partir de fichiers de mesures **CSV** ou **Excel**,
de façon rapide et interactive. Le principe :

1. on **charge** un ou plusieurs fichiers (page *File Select*) ;
2. on **ajoute les graphes un par un** (page *Plots*) en choisissant X, Y, des
   filtres et des séries ;
3. on règle la **mise en page** (titres, polices, graduations, légende…) ;
4. on **exporte** en PowerPoint, en images PNG ou en CSV (page *Export*).

L'interface est organisée en pages, accessibles par la barre de navigation à
droite de la fenêtre :

| Page | Rôle |
|---|---|
| **File Select** | charger et gérer les fichiers de données |
| **Data** | consulter le contenu brut d'un fichier |
| **Functions** | créer des colonnes calculées (formules) |
| **Plots** | créer et régler les graphes |
| **Export** | exporter en PowerPoint / PNG / CSV |
| **Help** | aide-mémoire intégré |

La barre du haut contient le menu **File** (ouvrir des fichiers, ouvrir /
enregistrer un espace de travail, charger une police) et le bouton
**QuickSave**. La barre du bas affiche les messages (chargement terminé,
erreurs…) et la mémoire utilisée.

# 2. Installation

1. Sur GitHub, ouvrir la page **Releases** du dépôt
   (`https://github.com/Citroz31/CurveTracer/releases`).
2. Télécharger **`CurveTracer.exe`** (Windows) — ou l'archive
   `CurveTracer-windows-x64.zip`, qui contient aussi ce guide et un fichier
   d'exemple.
3. Lancer `CurveTracer.exe`. Aucune installation n'est nécessaire.

> Windows peut afficher « Windows a protégé votre ordinateur » (SmartScreen)
> car l'exécutable n'est pas signé : cliquer sur **Informations
> complémentaires → Exécuter quand même**.

Raccourcis utiles : **Ctrl+O** ouvrir des fichiers, **Ctrl+S** enregistrer
l'espace de travail.

# 3. Charger des données (page *File Select*)

![Page File Select](screenshots/files.png)

- **Open data files…** (ou glisser-déposer sur la fenêtre) : formats
  `csv`, `txt`, `tsv`, `dat`, `xlsx`, `xlsm`, `xlsb`, `xls`, `ods`.
  Plusieurs fichiers peuvent être chargés en même temps.
- La détection est **automatique** :
  - délimiteur (`,` `;` tabulation `|`) ;
  - **ligne d'en-tête** : les lignes d'information placées au-dessus des noms de
    colonnes (Test Request ID, Part Number…) sont ignorées et visibles dans
    **File information** ;
  - virgule décimale (`1,5`), valeurs `N/A`, numéros de série avec zéros
    (`07102601`) conservés tels quels.
- Chaque fichier chargé apparaît dans **Project Data Sources** avec son nombre
  de lignes, de colonnes, de graphes, et les boutons :
  - **Add Plot** : crée un graphe sur ce fichier ;
  - **View data** : ouvre la page *Data* ;
  - **Download Data** : enregistre les données en CSV ;
  - **⚙ Options** : force le délimiteur, la ligne d'en-tête (numéro de ligne,
    0 = pas d'en-tête), la virgule décimale ou la feuille Excel, puis
    **Apply & reload** ;
  - **⟳** recharger le fichier, **🗑** le retirer du projet.
- **Import options (for the next files)** : mêmes réglages, appliqués aux
  prochains fichiers ouverts.

# 4. Consulter les données (page *Data*)

![Page Data](screenshots/data.png)

Choisir le fichier dans la liste déroulante. La table affiche toutes les lignes
(même plusieurs millions, le défilement reste fluide). Le champ **Columns**
filtre les colonnes par nom. Survoler un en-tête de colonne affiche son type,
son nombre de valeurs uniques et son min / max. Les colonnes en bleu sont des
fonctions utilisateur.

# 5. Créer un graphe (page *Plots*)

Cliquer **Add Plot** sur un fichier (page *File Select*) ou **+ New plot** en
haut de la page *Plots*. Chaque graphe a son onglet (`1. S21 25°C`,
`2. …`) ; un clic droit sur l'onglet propose **Copy / Split Plot** (dupliquer)
et **Delete**.

![Page Plots](screenshots/plots.png)

La barre d'outils au-dessus du graphe contient : le **nom** du graphe, un
résumé (nombre de courbes, de points, de lignes retenues, temps de calcul), et
les boutons **⟲** (réinitialiser la vue), **Copy / Split Plot**, **PNG**
(image haute résolution), **Download Data** (courbes en CSV) et **🗑**.

Le panneau de droite contient quatre onglets : **Data**, **Layout**,
**Plot Options**, **Trace Options**.

## 5.1 Onglet Data → Select Data

- **Data source** : fichier utilisé par le graphe.
- **Columns** : liste des colonnes avec leur nombre de valeurs uniques
  (tri par nom ou par *Unique vals*, champ de recherche). Le symbole indique le
  type : `#` numérique, `Aa` texte, `#?` mixte. Pour utiliser une colonne :
  **la glisser** dans une des zones ci-dessous, ou cliquer sur
  **X**, **Y**, **F** (filtre) ou **S** (série).
- **X-Data (limit 1)** : la colonne en abscisse, avec son **Modifier** :

  | Modifier | Effet | Exemple |
  |---|---|---|
  | ÷ 1e9 (Hz → GHz) | divise par 10⁹ | `Freq` → `Freq (GHz)` |
  | ÷ 1e6 / ÷ 1e3 | divise par 10⁶ / 10³ | MHz, kHz |
  | × 1e3 (A → mA) | multiplie par 10³ | `Current [A]` → `Current [mA]` |
  | × 1e6 / × 1e9 | µ, n | |
  | Custom factor | multiplie par un facteur libre | |

  Une colonne de fréquence est automatiquement passée en GHz.
- **Y-Data** : une ou plusieurs colonnes (même choix de *Modifier*).
- **Filter** : ne garder que certaines lignes. Cliquer sur le filtre (▶) pour
  le déplier :
  - mode **Values** : cases à cocher par valeur, boutons **All**, **None**,
    **Invert**, recherche, et **only** pour ne garder qu'une valeur ;
  - mode **Range** : plage numérique `min … max` (bornes optionnelles).

  Le titre du filtre résume la sélection, ex. `Set_TempC (1/3)` = 1 valeur sur 3.
  La case à cocher active / désactive le filtre sans le supprimer.
- **Series** : **une courbe par valeur** de la colonne (SN, température,
  part number…). Plusieurs colonnes se combinent (`SN=…, Set_TempC=…`) ;
  **⬆** change l'ordre de regroupement.

![Filtre déplié](screenshots/filter_open.png)

## 5.2 Onglet Data → Process Data

![Process Data](screenshots/process.png)

Cocher **Apply a processing to the curves** puis choisir la **Function** :
moyenne (*Mean*), médiane, min, max, somme, **RMS**, écart-type, étendue
(max − min) ou comptage. Le calcul se fait **pour chaque valeur de X** :

- sans série cochée : les points ayant le même X dans chaque courbe sont combinés ;
- avec des séries : cocher les colonnes à **combiner** (*Combine the curves
  across*). Exemple : séries = SN + température, combiner sur SN → une courbe
  moyenne par température ;
- **Keep the original curves** : affiche aussi les courbes d'origine, en plus
  clair, sous les courbes calculées (en trait épais).

## 5.3 Onglet Data → View Data

Table des lignes retenues par les filtres (colonnes X, Y, séries et filtres ;
cocher **All columns** pour tout voir).

## 5.4 Onglet Layout

![Onglet Layout](screenshots/layout.png)

- **Title** : afficher ou non un titre, son texte, sa police / taille / couleur.
- **Axis labels** : libellés X et Y (vide = automatique, à partir du nom de
  colonne et du *Modifier*), police des libellés, police des graduations
  (*Tick labels font*).
- **X axis / Y axis** :
  - **Automatic range** décoché : plage d'affichage `from … to` ;
    **Use current zoom** recopie la vue actuelle ;
  - **Automatic ticks** décoché : **pas des graduations** (ex. 5 GHz, 2 dB) ;
  - **Automatic decimals** décoché : nombre de décimales des graduations ;
  - **Grid** : grille.
- **Legend** : affichage, position (intérieur / extérieur, en haut, à droite…),
  titre, police, fond, bordure.
- **Colors** : couleur de fond et de grille.
- **Apply this layout to all plots** : recopie polices, légende, grille et
  couleurs sur tous les graphes (titres, libellés et plages sont conservés).

Les polices proposées : *Proportional*, *Monospace*, et les polices installées
(Arial, Calibri, Segoe UI, Times New Roman, Courier New, Consolas…).
**File → Load a font file** ajoute n'importe quel `.ttf` / `.otf`.

## 5.5 Onglet Plot Options

- **Sort the points by X** : trie les points de chaque courbe ;
- **Start a new line when X goes backwards** : un nouveau balayage (ex. une
  autre tension non filtrée) démarre un nouveau trait au lieu de tracer un
  retour ;
- **Fast drawing of huge curves** : décimation min/max (sans perte visible) ;
- **Show the values under the mouse**, **Crosshair** ;
- **Markers** : liste des curseurs posés (voir § 6) ;
- **Export** : inclure le graphe dans les exports, titre de diapositive.

## 5.6 Onglet Trace Options

![Onglet Trace Options](screenshots/traces.png)

- **Defaults** : palette de couleurs, épaisseur, style de trait (plein, tirets,
  pointillés, aucun), marqueurs et leur taille ; **Apply these defaults to all
  plots**.
- **Curves** : pour chaque courbe — visibilité, couleur, épaisseur, style,
  marqueur et **libellé de légende personnalisé**. **Show all / Hide all /
  Reset** agissent sur toutes les courbes.

# 6. Explorer un graphe

| Action | Effet |
|---|---|
| Molette | zoom autour de la souris |
| Glisser (clic gauche) | déplacer la vue |
| Clic droit + glisser | zoom sur un rectangle |
| Glisser sur un axe | zoom sur cet axe seulement |
| Double-clic | revenir à la vue initiale |
| Survol | valeurs x / y de la courbe la plus proche |
| **Clic sur une courbe** | pose un **curseur** avec ses valeurs x / y |
| Clic sur un curseur | le supprime |
| Clic droit | menu : *Reset view*, *Clear markers* |

**La légende** se déplace à la souris. Déposée **contre un bord** du graphe,
elle passe **à l'extérieur du cadre** et le graphe se réduit automatiquement
pour lui laisser la place. Un clic sur une entrée masque / affiche la courbe
correspondante. Quand il y a beaucoup de courbes, la légende se répartit sur
plusieurs colonnes.

![Légende placée à l'extérieur, curseur posé](screenshots/legend_out.png)

# 7. Fonctions utilisateur (page *Functions*)

![Page Functions](screenshots/functions.png)

Une fonction crée une **nouvelle colonne** calculée à partir des autres. Elle
est disponible dans tous les fichiers qui contiennent les colonnes utilisées,
et s'utilise comme une colonne normale (X, Y, filtre, série). Elle apparaît
dans la liste des colonnes sous **User Functions**.

- **+ New function**, saisir le **nom** et l'**expression**, puis **✔ Apply**
  (ou quitter le champ). La colonne *Status* indique dans combien de fichiers
  la fonction est valide (survoler pour voir les erreurs).
- Colonnes : entre crochets `[VDD Supply Current [A]]`, ou directement si le
  nom est simple (`S21db`, `Freq`). Cliquer une colonne dans *Available
  columns* copie `[nom]` dans le presse-papier.
- Opérateurs : `+ - * / ^`, parenthèses, constantes `pi` et `e`.
- Fonctions : `abs sqrt exp ln log10 log2 sin cos tan asin acos atan atan2
  deg rad floor ceil round sign min max pow hypot`, et pour la RF :
  `db10(x)` = 10·log10(x), `db20(x)` = 20·log10(|x|), `lin10(x)` = 10^(x/10),
  `lin20(x)` = 10^(x/20), `dbm2w(x)`, `w2dbm(x)`.

Exemples :

| Nom | Expression |
|---|---|
| Frequency (GHz) | `[Freq] / 1e9` |
| Gain - Thru Loss | `[S21db] - [Thru Loss]` |
| NF + Input Loss | `[NF] + 0.8` |
| DC power (W) | `[VDD Supply Voltage [V]] * [VDD Supply Current [A]]` |

# 8. Exporter (page *Export*)

![Page Export](screenshots/export.png)

- Les **miniatures** de tous les graphes sont affichées : la case à cocher
  inclut / exclut un graphe, le champ texte donne le **titre de la diapositive**
  (vide = titre ou nom du graphe), la croix (🗙) retire le graphe de l'export,
  double-clic ouvre le graphe.
- **Slide format** : *Plot only* ou *Plot + Settings* (ajoute à droite la
  source, X, Y, séries, filtres et traitement).
- **Image size** : résolution des images (1600 × 1000 par défaut).
- **Use the current zoom of each plot** : exporte la vue affichée plutôt que la
  plage complète.
- **File name**, puis :
  - **Download PowerPoint** : un fichier `.pptx`, une diapositive 16:9 par graphe ;
  - **Download Images (PNG)** : une image par graphe dans le dossier choisi ;
  - **Download Data (CSV)** : les points de chaque graphe (colonnes *Trace, X, Y*).

![Diapositive générée](screenshots/slide-1.png)

# 9. Espaces de travail

**File → Save workspace**, **QuickSave** ou **Ctrl+S** enregistre dans un
fichier `.ctw` : la liste des fichiers de données et leurs options, les
fonctions utilisateur, tous les graphes et leurs réglages, et les options
d'export. **File → Open workspace** (ou glisser-déposer le `.ctw`) recharge le
tout ; les fichiers de données sont relus depuis leur emplacement d'origine.

Astuce : associer l'extension `.ctw` à `CurveTracer.exe` dans Windows permet
d'ouvrir un projet par double-clic.

# 10. Questions fréquentes

**Le graphe affiche « Select an X column and at least one Y column ».**
Choisir X et au moins un Y dans *Data → Select Data*.

**Aucune courbe n'apparaît / « No row matches the filters ».**
Un filtre exclut toutes les lignes : vérifier les valeurs cochées (`(0/3)`).

**Les courbes font des allers-retours.**
Plusieurs balayages sont mélangés dans une même série : ajouter un filtre ou une
série sur la colonne qui varie (tension, état…), ou cocher *Sort the points by X*.

**Les colonnes sont mal découpées / l'en-tête est faux.**
Dans *File Select → ⚙ Options*, forcer le délimiteur ou le numéro de la ligne
d'en-tête, puis *Apply & reload*.

**Les nombres avec une virgule ne sont pas reconnus.**
*⚙ Options → Decimal comma → Yes*.

**La légende cache les courbes.**
La déplacer à la souris, ou la poser contre le bord droit / bas pour la sortir
du cadre (ou *Layout → Legend → Position*).
