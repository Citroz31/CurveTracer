## Télécharger

| Fichier | Contenu |
|---|---|
| **CurveTracer.exe** | l'application Windows (64 bits), à lancer directement — aucune installation |
| **CurveTracer_Guide_utilisateur.pdf** | le guide utilisateur illustré |
| CurveTracer-windows-x64.zip | exécutable + guide + fichier d'exemple |
| CurveTracer-linux-x64.zip / CurveTracer-macos.zip | versions Linux et macOS |

> Au premier lancement, Windows SmartScreen peut afficher « Windows a protégé votre ordinateur » tant que l'exécutable n'est pas signé :
> **Informations complémentaires → Exécuter quand même**, ou une fois pour toutes *clic droit → Propriétés → Débloquer*.
> Les étapes pour supprimer définitivement cet avertissement (signature de code) sont décrites dans `docs/SIGNATURE_WINDOWS.md`.
> `SHA256SUMS.txt` donne l'empreinte de chaque fichier (vérification, liste blanche du service informatique).

## Nouveautés de la version 0.3.5

- **Lissage nettement meilleur sur les courbes à peu de points** : nouvelle **spline de lissage** (méthode par défaut, un seul réglage « Smoothness »)
  et **LOESS** (régression locale, option robuste) ; la courbe lissée est tracée comme une **courbe continue** et non plus en segments
  entre les points (option *Smooth curve between the points*, pour toutes les méthodes).
- **Liste des graphes façon PowerPoint** : clic n'importe où sur la carte, **glisser-déposer** pour réordonner, **clic droit** pour
  dupliquer / déplacer / supprimer, touches **Suppr** et **Ctrl+D**.
- **Miniatures fidèles** : mêmes proportions et même mise en page que le graphe réel (et que l'image exportée dans la page Export).
- **Annuler / rétablir (Ctrl+Z / Ctrl+Y)** : curseurs posés ou retirés, graphes supprimés, dupliqués ou déplacés ; boutons ↶ ↷.

## Nouveautés de la version 0.3.4

- Exécutable Windows avec **icône**, **informations de version et d'éditeur** (*Propriétés → Détails*) et manifeste standard (droits utilisateur, chemins longs) ; icône de la fenêtre.
- **Signature de code prête** : la chaîne de compilation signe `CurveTracer.exe` (Azure Artifact Signing) dès que les identifiants sont configurés — voir `docs/SIGNATURE_WINDOWS.md`.
- Empreintes SHA-256 publiées avec chaque version.

## Nouveautés de la version 0.3.3 — fiabilité

- **Plantage corrigé** : l'onglet *View Data* pouvait fermer l'application après le rechargement d'un fichier plus court.
- **Plantage corrigé** : une feuille Excel vide avec ligne d'en-tête forcée bloquait le chargement (« Loading… » sans fin) et désactivait la détection des modifications ; tout incident de lecture est maintenant signalé comme une erreur.
- **Fermeture de la fenêtre** : CurveTracer propose d'enregistrer les modifications (*Save / Don't save / Cancel*).
- Le type d'une colonne ne dépend plus du nombre de cœurs du PC (même résultat partout pour un même fichier).
- Supprimer un graphe placé au-dessus du graphe affiché ne change plus de graphe.
- De-embedding et fonctions utilisateur recalculés **en tâche de fond** (plus de gel de la fenêtre sur les gros fichiers) ; un rechargement / de-embedding / changement de fonction **garde le zoom** des graphes.
- Quitter un champ de fonction sans l'avoir modifié ne recalcule plus tous les graphes.
- Trackpad : un geste = un graphe (l'inertie ne fait plus sauter plusieurs graphes).
- Polices chargées par l'utilisateur propres à chaque workspace ; écran d'accueil sans accès disque à chaque image ; mémoire de la barre d'état mise en cache ; grosses courbes non triées décimées à l'affichage.

## Nouveautés de la version 0.3.2

- **Lissage des courbes** (Data → Process Data → *Smoothing*) : moyenne glissante, gaussien, **Savitzky–Golay**, médiane (anti-pics), exponentiel sans déphasage, moyenne sur une largeur en X ;
  fenêtre / σ / ordre / nombre de passes réglables, préréglages, courbes brutes conservées en clair.
- **Correctif** : le panneau de réglages ne s'élargit plus tout seul (sa largeur ne dépend plus que de sa bordure ; un contenu trop large défile).
- **Filtres sur des milliers de valeurs** (ex. 4096 états d'un corechip) : liste virtualisée, recherche avec *Select / Unselect matches*,
  calcul du filtre en temps linéaire, clé de calcul sans formatage du texte, un seul calcul à la fois par graphe (plus d'empilement de calculs en cliquant vite).
  Coût d'affichage avec le filtre ouvert : ~42 ms → ~17 ms par image sur un fichier de 150 Mo.
- Miniatures allégées pour les très gros graphes ; regroupement des agrégations en temps linéaire.

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
