# web_pluie

Carte radar des précipitations et **probabilité qu'il pleuve sur votre position GPS**, de 5 min à 1 h.
Pensée pour le téléphone. Inspirée de [web_weather](https://github.com/pilou33620/web_weather), dont elle reprend le serveur.

- **Fond d'écran qui suit la météo** (repris du « Paysage » de tooth_dentaire) : ciel selon l'heure (aube, jour, crépuscule, nuit,
  d'après le lever et le coucher du soleil) et le temps (dégagé, nuageux, couvert, brouillard, pluie, neige, orage), soleil ou lune,
  nuages qui dérivent, maison aux fenêtres allumées ; pluie, neige ou étoiles animées, éclairs par orage. La pluie et la neige
  viennent du radar sur votre position, la nébulosité et la température d'AROME. Animations coupées si le système demande moins de mouvement.
- **Commune, température et temps actuel** en grand, sur le ciel ; **cumuls** : pluie attendue d'ici 1 h (médiane et
  80e centile de l'ensemble) et pluie tombée ces 2 dernières heures d'après le radar.
- **Grêle et averses violentes** (écho de 55 dBZ ou plus) : bandeau d'alerte avec la probabilité d'ici 1 h, cellules marquées
  « grêle possible » sur la carte, notification.
- **En mouvement** (à pied, à vélo, en voiture : vitesse et cap du GPS), la probabilité suit votre trajet supposé, tout droit à la
  vitesse actuelle, tracé en pointillés sur la carte.
- **Phrase de synthèse** : « Pluie probable dans ~20 min (70 %) », « Il pleut sur vous, fin probable dans ~15 min »…
- **Trois tuiles** : risque de pluie d'ici 15 min, 30 min et 1 h.
- **Courbe 5 → 60 min** : probabilité à chaque minute, et probabilité d'avoir de la pluie au moins une fois d'ici là ;
  intensité attendue (mm/h) dans un second graphique ; tableau des valeurs.
- **Carte** : les 2 dernières heures de radar puis la pluie prévue jusqu'à +1 h, avec un curseur et la lecture animée ;
  en option, les flèches de déplacement et les averses fortes suivies (contour, trajectoire passée et prévue).
- **Notifications** : quand le risque d'ici 30 min dépasse un seuil réglable (30 à 90 %), quand la pluie va s'arrêter (en option),
  et en cas de risque de grêle.
- **Fiabilité dans la page** : scores par échéance, comparaison avec la persistance, et le réglage automatique en cours.
- **Icône d'écran d'accueil** (PNG pour iPhone et Android) : l'appli s'installe comme une vraie appli.

Les probabilités sont **calculées par l'outil** à partir des images radar : ce sont des estimations, pas des prévisions officielles.

## Les algorithmes de prévision

RainViewer ne fournit plus de prévision radar depuis 2026 : tout est calculé ici, dans `rust/pluie/src/`.

| Étape | Fichier | Principe |
|---|---|---|
| Décodage | `radar.rs` | Les tuiles brutes (palette « Universal Blue », sans lissage) sont relues en réflectivité (dBZ) par la table inverse de la palette. Mosaïque de 3 × 3 tuiles au zoom 7 autour de vous : ~660 km de côté, ~0,9 km par pixel. Pluie à partir de 12 dBZ (~0,2 mm/h, Marshall-Palmer). |
| Flux optique | `mouvement.rs` | Appariement de blocs de ~55 km entre images successives (corrélation normalisée cumulée sur les 4 dernières images, soit 30 min), affiné au sous-pixel, puis lissé en champ de vecteurs ; le mouvement d'ensemble comble les zones sèches. |
| Advection | `mouvement.rs` | Semi-lagrangienne : pour savoir ce qui sera en un point dans t minutes, on remonte la trajectoire de t minutes (méthode du point milieu) et on lit l'image actuelle. Sert aux images de prévision de la carte. |
| Suivi de cellules | `cellules.rs` | Les averses à 35 dBZ ou plus sont repérées comme objets (composantes connexes), appariées d'une image à l'autre et suivies. Une cellule suivie sur 3 images ou plus impose sa vitesse propre au champ autour d'elle. |
| Ensemble | `prevision.rs` | 40 membres : vitesse perturbée (±15 %), croissance ou déclin tirés autour de la tendance mesurée, décalage qui grandit avec l'échéance (1 km + 0,1 km/min). Chacun remonte minute par minute la trajectoire qui arrive sur vous. La probabilité est la part des membres mouillés. |
| AROME | `prevision.rs`, `sources.rs` | Part des 9 points voisins (~10 km) où AROME (France, 2,5 km) fait pleuvoir, au pas de 15 min. Mélangée au radar à partir de 20 min d'échéance, avec un poids qui grandit jusqu'à la moitié vers 80 min. |
| Grêle, cumul | `prevision.rs` | Part des membres qui voient un écho de 55 dBZ ou plus sur le point ; cumul de pluie de chaque membre (somme des intensités), dont on donne la médiane et le 80e centile. |
| En mouvement | `prevision.rs` | Le point suit le trajet (vitesse et cap du GPS) : pour chaque minute, la trajectoire de la pluie est remontée depuis l'endroit où vous serez. |
| Réglage automatique | `calibration.rs` | D'après le journal, relu toutes les heures : poids du radar face à AROME qui aurait donné le meilleur score (par tranche d'échéance, dès 100 vérifications avec AROME), puis recalibrage par classes de probabilité (si les « 70 % » n'ont plu que 5 fois sur 10, les prochains sont ramenés vers 50 %, d'autant plus que les vérifications sont nombreuses). |
| Vérification | `journal.rs` | Chaque prévision est confrontée au radar observé ensuite sur le même point (`donnees/verification.jsonl`). `pluie --bilan` (ou `bin/pluie --bilan`) imprime le score de Brier par échéance, pour le mélange, le radar seul, AROME seul et la persistance, avec la détection et les fausses alertes. |

Les incertitudes de l'ensemble sont fixées à la main ; le réglage automatique corrige ensuite les probabilités d'après ce qui s'est
vraiment passé. En mouvement, la prévision n'est pas vérifiée (le point change).

## Sources

Le navigateur ne parle qu'au serveur, qui relaie et garde en cache :

| Source | Usage | Cache |
|---|---|---|
| [RainViewer](https://www.rainviewer.com) (usage personnel) | radar de pluie, 2 h passées toutes les 10 min, zoom 7 au plus, 100 requêtes par minute au plus | liste 2 min, tuiles 2 h |
| [Open-Meteo](https://open-meteo.com) (CC BY 4.0) | AROME au pas de 15 min sur 9 points autour de vous, et le ciel actuel (nébulosité, température, lever et coucher du soleil) pour le fond d'écran, en un seul appel | 15 min |
| [OpenStreetMap](https://www.openstreetmap.org/copyright) | fond de carte | disque, 7 jours (`cache/`) |
| [API Adresse](https://adresse.data.gouv.fr) (Licence ouverte) | nom de la commune (position arrondie à ~1 km) | 30 jours |

## Installation

### Le plus simple : l'archive de production (sans git, sans rien installer)

Dans la [publication « binaires »](https://github.com/pilou33620/web_pluie/releases/tag/binaires), prendre l'archive du système :
`pluie-prod-windows-x86_64.zip`, `pluie-prod-linux-x86_64.tar.gz`, `pluie-prod-linux-aarch64.tar.gz` (Raspberry Pi 64 bits)
ou `pluie-prod-macos-aarch64.tar.gz`. La décompresser, puis double-cliquer sur `demarrer_WEB_PLUIE.cmd` (Windows) ou lancer
`./demarrer_WEB_PLUIE.sh`. À chaque démarrage, l'outil compare son archive à la dernière publiée et se met à jour
(empreinte SHA-256 vérifiée). Les données (`cache/`, `donnees/`) ne sont pas dans l'archive : une mise à jour n'y touche jamais.

### Pour développer : le dépôt git

```sh
git clone https://github.com/pilou33620/web_pluie.git
cd web_pluie
sh demarrer_WEB_PLUIE.sh  # Raspberry Pi, Linux, macOS ; Windows : double-clic sur demarrer_WEB_PLUIE.cmd
```

Rust n'est pas nécessaire : sans `cargo`, `scripts/binaires.sh` télécharge l'exécutable compilé par la CI (empreinte
SHA-256 vérifiée). Avec `cargo`, il est compilé sur place.

## Options

| Option | Rôle |
|---|---|
| `--port N` | port d'écoute (8802 par défaut ; le suivant libre s'il est pris) |
| `--local` | n'écouter que sur `127.0.0.1` (pas d'appairage) |
| `--sans-navigateur` | ne pas ouvrir le navigateur |
| `--sans-pause` | acceptée pour le launcher |
| `--sans-maj` | ne pas vérifier les mises à jour au démarrage |
| `--bilan` | imprimer le bilan de fiabilité des prévisions, puis quitter |

## Utiliser depuis le téléphone

Le terminal affiche l'adresse réseau (`https://IP:8802`) et un **code d'appairage** à 6 chiffres, demandé une fois par appareil.
**Le GPS du navigateur exige une connexion sécurisée** : installer une fois la CA du poste sur le téléphone (lien sur la page
d'appairage, `/appairage/ca.pem`), sinon la carte s'affiche mais pas la probabilité sur votre position. Pour les notifications sur
iPhone, ajouter l'appli à l'écran d'accueil.

## Mises à jour

Clone git : au démarrage, `git fetch` puis avance rapide seulement (`--ff-only`), et redémarrage automatique (code 75). Sous
web_launcher, c'est le launcher qui met à jour (`--sans-maj`). Copie de production : voir plus haut.

## Données

`cache/` (tuiles OSM) et `donnees/verification.jsonl` (journal de vérification, 5 Mo au plus, plus un ancien fichier `.1`), ignorés
par git. Le journal contient les positions arrondies au centième de degré (~1 km).

## Tests

```sh
cargo test --release -p pluie
node test/paysage.test.js   # fond d'écran : moment de la journée, temps choisi, décor
```

Sans réseau : décodage d'une vraie tuile RainViewer, flux optique et advection sur une pluie synthétique au mouvement connu,
suivi de cellules, ensemble (front qui arrive, pluie qui faiblit), AROME et ciel, journal et bilan, et le serveur de bout en bout (fichiers
servis et refusés, Host étranger, prévision complète sur de fausses sources). Le Raspberry Pi ne compile pas : la CI GitHub
(`ci.yml`) lance les essais, `binaires.yml` publie les exécutables.

## Limites connues

- Hors de la couverture des radars, pas d'écho veut dire « pas de pluie » : l'outil ne sait pas distinguer l'absence de données.
- Le radar ne voit pas naître une averse : une cellule qui se forme sur place n'est prévue que par la part AROME.
- Les prévisions en attente de vérification vivent en mémoire : un redémarrage perd celles de l'heure et demie en cours.
- Une averse plus rapide que ~165 km/h n'est pas suivie par le flux optique.
