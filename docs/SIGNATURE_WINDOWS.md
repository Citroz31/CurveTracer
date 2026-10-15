# Supprimer l'avertissement Windows SmartScreen

## Pourquoi Windows affiche « Windows a protégé votre ordinateur »

Un `.exe` téléchargé sur Internet porte une marque « provenant d'Internet ».
Windows SmartScreen bloque alors tout exécutable qui n'a pas de
**réputation** : il ne connaît ni le programme, ni son éditeur. Aucun réglage
du code ne peut l'éviter. Il faut que l'exécutable soit **signé
numériquement** par un éditeur identifié, ou distribué par le
**Microsoft Store**.

Depuis 2024, même un certificat « EV » ne donne plus de réputation immédiate :
un exécutable signé construit sa réputation au fil des téléchargements. La
signature reste indispensable : sans elle, la réputation repart de zéro à
chaque version.

CurveTracer est prêt pour la signature : l'exécutable contient son icône, ses
informations de version (*Propriétés → Détails*) et un manifeste standard. La
chaîne de compilation GitHub signe automatiquement `CurveTracer.exe` dès que
les secrets ci-dessous sont configurés.

## Option 1 : Azure Artifact Signing (signature automatique de chaque version)

Service de Microsoft (ex-« Trusted Signing »), environ 10 $/mois,
certificats gérés et renouvelés par Microsoft.

**Conditions** : une **organisation** (entreprise, association…) de l'UE, du
Royaume-Uni, des États-Unis, du Canada… En Europe, les **particuliers ne sont
pas éligibles** à ce jour (réservé aux particuliers des États-Unis et du
Canada).

1. Créer un abonnement Azure, puis une ressource **Artifact Signing
   Account** (région proche, ex. *West Europe*). Noter son **endpoint**
   (ex. `https://weu.codesigning.azure.net/`) et son **nom**.
2. Dans le compte : **Identity validation** → *Public* → organisation
   (documents de l'entreprise). Compter quelques jours.
3. Créer un **Certificate profile** de type *Public Trust* lié à cette
   validation et noter son nom.
4. Microsoft Entra ID → **App registrations** → *New registration*
   (ex. `curvetracer-signing`) ; créer un **client secret**. Noter le
   *Directory (tenant) ID*, l'*Application (client) ID* et le secret.
5. Sur le compte Artifact Signing → *Access control (IAM)* : donner à cette
   application le rôle **Artifact Signing Certificate Profile Signer**.
6. Sur GitHub : *Settings → Secrets and variables → Actions → New repository
   secret*, créer :

   | Secret | Valeur |
   |---|---|
   | `AZURE_TENANT_ID` | Directory (tenant) ID |
   | `AZURE_CLIENT_ID` | Application (client) ID |
   | `AZURE_CLIENT_SECRET` | le client secret |
   | `SIGNING_ENDPOINT` | endpoint du compte |
   | `SIGNING_ACCOUNT_NAME` | nom du compte |
   | `SIGNING_CERTIFICATE_PROFILE` | nom du certificate profile |

Les prochaines versions seront signées automatiquement. Le journal de la CI
(étape *Show the signature*) affiche `Status : Valid`. Dans Windows, *clic
droit → Propriétés → Signatures numériques* montre l'éditeur.

## Option 2 : Microsoft Store (aucun avertissement)

Les applications du Store sont signées par Microsoft et ont une réputation
complète.

- **Compte particulier gratuit**, vérification par pièce d'identité. Il est
  réservé à un usage **non commercial**. Pour une diffusion dans le cadre
  professionnel, il faut un compte **entreprise** (payant).
- L'exécutable doit être empaqueté au format **MSIX** avec l'identité donnée
  par le Partner Center. Cette étape peut être ajoutée à la CI une fois le nom
  de l'application réservé.

## Option 3 : certificat de signature acheté auprès d'une autorité

Exemples : Certum, Sectigo, DigiCert, SSL.com. Il en existe pour les
particuliers en Europe. Les clés sont fournies sur un jeton matériel ou dans
un service de signature en ligne : la signature dans la CI dépend du service
choisi. La réputation SmartScreen se construit ensuite progressivement.

## En attendant : utilisation en entreprise

- **Une seule fois par fichier téléchargé** : *clic droit sur
  `CurveTracer.exe` → Propriétés → cocher « Débloquer » → OK*, ou en
  PowerShell : `Unblock-File .\CurveTracer.exe`. Windows n'affiche plus
  l'avertissement pour ce fichier.
- Le service informatique peut autoriser CurveTracer par son empreinte
  (SHA-256) ou le déployer via Intune ou une GPO. Ces méthodes ne déclenchent
  pas SmartScreen.
