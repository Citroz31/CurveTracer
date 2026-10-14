## Télécharger

| Fichier | Contenu |
|---|---|
| **CurveTracer.exe** | l'application Windows (64 bits), à lancer directement — aucune installation |
| **CurveTracer_Guide_utilisateur.pdf** | le guide utilisateur illustré |
| CurveTracer-windows-x64.zip | exécutable + guide + fichier d'exemple |
| CurveTracer-linux-x64.zip / CurveTracer-macos.zip | versions Linux et macOS |

> Au premier lancement, Windows SmartScreen peut afficher « Windows a protégé votre ordinateur » (exécutable non signé) :
> **Informations complémentaires → Exécuter quand même**.

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
