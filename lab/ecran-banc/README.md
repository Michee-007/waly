# Banc B — agent d'écran (2026-09-11)

Plan : `docs/PLAN-2026-09-11-B-agent-ecran.md`. Crate autonome hors workspace,
UI Automation = COM sur `uiautomationcore.dll` (DLL système signée) via la
crate `windows` 0.61, cross-compilée depuis WSL (`x86_64-pc-windows-gnu`,
profil dev). Rien n'est écrit sur disque ; seules les statistiques des
fenêtres de l'utilisateur sont relevées (jamais leur contenu).

```
# depuis WSL
cd /mnt/c/waly/lab/ecran-banc && CARGO_TARGET_DIR=~/waly-target-ecran-banc \
  cargo build --target x86_64-pc-windows-gnu
cp ~/waly-target-ecran-banc/x86_64-pc-windows-gnu/debug/ecran-banc.exe .
# côté Windows
powershell -ExecutionPolicy Bypass -File cible-banc.ps1   # fenêtre cible À NOUS (réduite)
ecran-banc arbre --titre "Cible banc Waly" --hors-ecran --brut
ecran-banc agir --titre "Cible banc Waly" --hors-ecran --nom "enregistrer" --action invoquer
ecran-banc regarde-moi --secondes 30     # GATE 3 : À LANCER PAR L'UTILISATEUR
```

## Verdicts

### SAC
- 1ᵉʳ build debug : passe. 2ᵉ build (réécriture) : **bloqué** (CodeIntegrity
  3033/3077) → `SAC_REROLL` = 1 → passe. Piège 3 tel quel, remède tel quel.
- Hooks bas niveau `WH_MOUSE_LL` + `WH_KEYBOARD_LL` : posés/retirés sous SAC
  en 0,01 ms (`regarde-moi --sonde`, aucun événement observé).

### GATE 1 — Lire ✅
Instantané = UN aller-retour cross-process (`CacheRequest`, `TreeScope_Subtree`,
filtre ControlView) → élagage (interactifs nommés, textes, conteneurs nommés)
→ rendu texte indenté, ids `[n]` sur les actionnables. Budget 5 000 car.
(~1 430 tok) : compaction des frères répétés (k = 8 → 5 → 3 → 2, « … +N
autres ») puis troncature honnête.

| Fenêtre (machine de Michée, en usage) | éléments vus | plein → rendu | tokens | temps |
|---|---|---|---|---|
| Bloc-notes (charte.md) | 66 | 1 992 car. complet | ~569 | 144-169 ms |
| Edge (PDF) | 38 | 889 car. complet | ~254 | 69-99 ms |
| Claude (Electron) | 177 | 6 784 → 4 384 (k=8) | ~1 252 | 51 ms |
| Chrome (nouvel onglet) | 149 | 7 012 → 4 054 (k=8) | ~1 158 | 25-88 ms |
| Chrome (page DeepL) | 550 | 26 023 car. **avant** compaction | ~7 435 | 47-54 ms |
| Zoom Meeting (pire cas) | 3 570 | 16 804 → 4 581 (k=8) | ~1 308 | 384-445 ms |
| Cible banc (WinForms, réduite) | 16 | 486 car. complet | ~138 | 65-84 ms |

- Critère < 300 ms fenêtre usuelle : **tenu** (25-169 ms). Zoom (3 570
  éléments) : 384-445 ms — hors « usuel », acceptable pour un tour de ~3,5 s.
- Critère < ~1 500 tok : **tenu partout après compaction** (k=8 suffit).
- Naïf (un aller-retour par propriété) vs cache : Explorateur 19 ms vs 16-43 ms
  sur 8 éléments — non discriminant à si petite taille ; le cache est retenu
  pour les grands arbres (Zoom : 3 570 éléments en < 0,5 s).
- **Limites mesurées** : fenêtre RÉDUITE de l'Explorateur → 8 éléments (le
  contenu n'est pas exposé réduit) ; Paramètres (UWP réduite, suspendue) →
  1 élément. En usage réel B1 lit la fenêtre VISIBLE choisie — sans objet ;
  pour agir en arrière-plan, Win32/WinForms/Electron OK, UWP réduite non.
- Valeurs : les propriétés de pattern (`ValueValue`, `ToggleToggleState`)
  doivent être DANS le cache (le pattern seul ne suffit pas — vécu : champ
  rempli rendu vide au 1ᵉʳ essai).
- Mots de passe : `IsPassword` → valeur jamais lue, rendue `••••`.

### GATE 2 — Agir ✅
Sur la fenêtre cible **réduite** (jamais montrée, jamais activée) :

| Action | Voie | Latence | Cible au 1ᵉʳ plan | Réduite avant→après | Relu |
|---|---|---|---|---|---|
| saisir « Nom du fichier » | `Value.SetValue` | 30-85 ms | non | oui → oui | `compte-rendu.md` ✓ |
| basculer « Ajouter la date » | `Toggle` | 6,5-6,7 ms | non | oui → oui | (non cochée) ✓ |
| saisir « Format » (liste déroulante) | `Value.SetValue` | 4,7-5,0 ms | non | oui → oui | `Markdown` ✓ |
| invoquer « Enregistrer » | `Invoke` | 17-71 ms | non | oui → oui | libellé « Enregistre : compte-rendu.md (date=False, format=Markdown) » ✓ |
| saisir « Mot de passe » | — | — | — | — | **refusé** par le banc ✓ |

- Curseur : INCHANGÉ quand mesurable (l'utilisateur utilisait la machine
  pendant le banc — mesure marquée « non concluante » quand la souris
  bougeait déjà AVANT l'action). Les patterns UIA ne synthétisent aucune
  entrée ; seul le repli clavier (`SetFocus` + `SendInput` unicode) exige le
  focus — non déclenché ici.
- **Parité Hermes** (« arrière-plan sans voler le curseur ») : tenue sur
  Win32/WinForms par les patterns.

### GATE 3 — Regarder 🟠 (code prêt, terrain = Michée)
`regarde-moi` : hook souris → `ElementFromPoint` → « clic bouton
« Enregistrer » — fenêtre « … » » ; hook clavier → raccourcis (Ctrl/Alt+…)
et Entrée/Tab/Échap SEULEMENT — les caractères ne quittent jamais le
callback ; saisies relues dans le champ à la sortie du focus ; mot de passe →
« •••• (non retenu) ». **Non joué sur l'activité réelle de Michée** : ce
serait l'enregistrer sans session qu'il a déclenchée (règle du plan). Scénario
de sortie à jouer par lui : Bloc-notes → menu Fichier → taper une ligne →
Ctrl+S → nommer → Enregistrer.

### E2E LLM — `crates/waly-sight/examples/agent_ecran.rs` ✅
Même hôte que le desktop (`waly_sight::mains::HoteUia`), vrai tour LLM
(qwen3 instruct sur Ollama 11434, machine en usage, 1 Go de RAM libre),
fenêtre cible RÉDUITE, approbations explicites simulées (`--oui` = le clic
« Approuver »). Consigne : « écris bilan.md dans le nom du fichier, choisis le
format PDF, puis clique sur Enregistrer ».

| Essai | Résultat | Vécu → correctif |
|---|---|---|
| 1 | 2 gestes sur 3, puis « clique sur Enregistrer » dit à l'UTILISATEUR | consigne partagée `mains_ecran::CONSIGNE` : « TOI-MÊME, jusqu'au bout, ne lui demande jamais de cliquer » |
| 2 | « choisir » appliqué à la liste elle-même, sans option → 2 approbations pour rien | `agir_ecran` exige l'option dans `texte` pour une liste déroulante ; `uia` choisit par valeur (éditable) ou dépliage + option du même nom |
| 3 | **tâche complète** : 4 tours (18,7 / 11,4 / 11,6 / 8,5 s), 3 gestes approuvés exécutés en 43-205 ms, relu « Enregistre : bilan.pdf (date=True, format=PDF) » | le 4B a écrit « bilan.pdf » au lieu de « bilan.md » — **le libellé humain l'affichait AVANT exécution** (le garde-fou sert) → consigne « texte EXACT » |

Lectures UIA pendant l'E2E : 24-300 ms.
