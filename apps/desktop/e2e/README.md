# E2E desktop — piloter la vraie UI par CDP

WebView2 expose le protocole Chrome DevTools si on lance l'app avec :

```powershell
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS='--remote-debugging-port=9222'
$env:WALY_DB="$env:TEMP\waly-e2e-test.db"   # base JETABLE — ne pas polluer la vraie
Start-Process C:\waly\bin\waly-desktop.exe
node e2e-stream.js   # tour streamé complet (question d'heure, observe les deltas)
node e2e-stop.js     # bouton stop en plein flux + tour suivant sain
```

Les scripts utilisent le WebSocket natif de Node (>= 21) — zéro dépendance.
Ils tapent dans le DOM réel (input + clic sur Envoyer) et sortent 0 si OK.
Prérequis : FLM qwen3-it:4b sur 127.0.0.1:42626.

⚠ **Si une autre instance de l'app tourne déjà** (celle de Michée), WebView2
PARTAGE son processus navigateur et ignore le port de debug. Isoler
l'instance de test :
`$env:WEBVIEW2_USER_DATA_FOLDER="$env:TEMP\waly-wv2-test"` (vécu 2026-07-07).

Écrits pendant R3 ch. 2 (2026-07-07). Si l'UI change d'ids (`input`, `send`,
`mstate`), les adapter.

## Lot 3 (2026-10-01) — jouables hors ligne

```powershell
node e2e-reflexion.js    # bloc « Réflexion » rempli puis replié, gardé au rechargement
node e2e-exterieur.js    # modèle extérieur = l'Ollama local en « serveur personnel », clé factice :
                         # filtre des fichiers joints, routeur, journal, bascule de cerveau local
node e2e-passerelle.js   # faux service Telegram local : appairage par code, intrus ignoré, réponse
```

Aucun ne touche un vrai fournisseur ni ne demande de vraie clé. Moteur requis :
Ollama sur 11434 avec `qwen3vl-it:4b` (et `llama3.2:3b` pour la bascule ;
`WALY_E2E_MODELE` / `WALY_E2E_AUTRE` pour d'autres noms). Les `.js` de ce
dossier sont des modules ES (`import`).

`e2e-partage.js` demande DEUX instances (bases et dossiers WebView2
distincts, CDP 9222 et 9223) et un relais local :
`waly-relais.exe 127.0.0.1:18787` (crate `waly-relais`).

## La Garde, étape 4 (2026-10-06) — l'enclos, sur un vrai agent

```powershell
node e2e-garde-enclos.js "<exe de l'agent>" "<dossier à donner>" [dossier des captures]
```

Passe par l'interface : choisit l'agent sur le graphe, « Mettre dans
l'enclos », essais, dossier donné puis repris, sortie de l'enclos, captures.
Prérequis : l'agent tourne, le compte `WalyEnclos` existe (`waly enclos
creer`). Plusieurs agents peuvent porter le même nom : le script ne clique
que si le panneau montre le programme demandé. `WALY_E2E_SURVEILLER=1` ajoute
la surveillance (Windows demande l'accord). Contrairement aux autres essais,
celui-ci a besoin de la vraie base ou d'une base qui porte le secret de
l'enclos (réglage `enclos_secret`, relisible par le même compte Windows).

## Refaire les captures du README (2026-10-06)

Sur la vraie app, base de démonstration vide (`WALY_DB`), fenêtre ramenée à
1500 × 1000 par CDP :

```powershell
node captures-readme.js <dossier>          # reflexion.png (un vrai tour du modèle local) et modeles.png
node captures-partage.js <fichier.png>     # deux instances (9222 et 9223) + waly-relais 127.0.0.1:18787
node captures-appel-video.js <fichier.png> # ALLUME caméra et micro ~20 s ; il faut quelqu'un devant
node capturer.js <fichier.png> [port] [expression JS] [attente ms]   # une capture, telle quelle
```

`garde.png` et `preuve.png` viennent de `e2e-garde-enclos.js`.

⚠ Vécu : un script qui AGIT sur un agent doit vérifier le programme affiché
et cliquer dans la même évaluation (`clicAgent`). Plusieurs agents portent le
même nom, et la page se redessine toutes les 2,5 s.
