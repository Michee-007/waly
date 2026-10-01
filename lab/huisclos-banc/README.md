# Banc R6a « Huis clos » — les 3 gates du scellé WFP

Prouve, mesure et tranche le mécanisme du scellé réseau AVANT le code produit
(plan : `docs/PLAN-2026-07-20-R6a-huis-clos.md`). Crate autonome hors workspace.

## Build & run

```
# depuis WSL
cd /mnt/c/waly/lab/huisclos-banc
CARGO_TARGET_DIR=~/waly-target-huisclos cargo build --target x86_64-pc-windows-gnu
cp ~/waly-target-huisclos/x86_64-pc-windows-gnu/debug/huisclos-banc.exe .

# cote Windows
huisclos-banc.exe banc          # auto-test complet (scelle SON PROPRE exe)
huisclos-banc.exe sonder        # sondes reseau seules
huisclos-banc.exe sceller <exe>... <secs>   # scelle des exes tiers N secondes
```

Le process eleve (UAC) a une console separee → le banc journalise aussi vers
`gate-eleve.log` (chemin fixe, `WALY_BANC_LOG` pour surcharger). Lancer eleve :
`Start-Process huisclos-banc.exe banc -Verb RunAs -Wait`.

## VERDICTS — les 3 gates VERTS (2026-07-20)

### GATE 1 — WFP depuis Rust windows-gnu sous SAC + modele de droits ✅
- L'exe **debug** (profil `dev` opt-level 1) passe SAC et pilote
  `fwpuclnt.dll` (DLL systeme signee Microsoft) via `windows-sys` — zero exe
  tiers, zero build-script C++ (piege n°3 respecte).
- **Utilisateur STANDARD** : `FwpmEngineOpen0` OK, mais
  `FwpmSubLayerAdd0` = **`0x80070005 ACCESS_DENIED`** (options moteur idem).
  → un utilisateur standard **ne peut pas** poser le sceau.
- **ELEVE** : toute la chaine OK (voir GATE 2/3).
- **Conclusion de droits** : le scelle **exige une elevation**. Trois voies
  pour eviter l'UAC a chaque scellement, a trancher AVEC Michee (posture de
  securite de SA machine) :
  1. **ACL one-time a l'installation** (voie recommandee a prototyper) :
     poser eleve UNE FOIS une sous-couche PERSISTANTE + provider Waly avec un
     descripteur de securite qui accorde au SID utilisateur le droit d'ajouter
     des filtres dedans ; ensuite le scelle se pose sans UAC. ⚠ le point dur
     est le droit `FWPM_ACTRL_ADD` sur l'OBJET LAYER (systeme, admin par
     defaut) en plus de `FWPM_ACTRL_ADD_LINK` sur la sous-couche → a mesurer
     en chantier 0.
  2. Service Windows installe une fois (plus de surface d'attaque).
  3. UAC a chaque scellement (fallback UX acceptable mais frottant).

### GATE 2 — PERMIT-loopback / BLOCK : le contrat exact ✅
Sous sceau (eleve, sur son propre exe) :
- loopback `127.0.0.1:<port>` → **CONNECTE en 0,2 ms** (la machinerie
  interne voix↔FLM↔desktop survivra intacte) ;
- `1.1.1.1:443`, `9.9.9.9:443` → **ECHEC `os error 10013`** (WSAEACCES,
  bloque par le pare-feu) ;
- UDP `8.8.8.8:53` → paquet drope (journalise), `send_to` local renvoie OK
  (fire-and-forget) mais rien ne sort → **sonder l'UDP par le journal, pas par
  le code de retour**.

### GATE 3 — cout/latence ✅ (toutes cibles tenues)
| Mesure | Valeur | Cible |
|---|---|---|
| Pose du sceau (4 filtres/exe) | **5,4 ms** | < 100 ms |
| Levee du sceau | **0,5 ms** | instantane |
| Mediane loopback ×200 sous sceau | **0,10 ms** (== baseline 0,10) | indistinguable |
| RAM / process resident | **0** (pas de process ni modele) | ~0 |

### Journal d'audit ✅
`FwpmNetEventEnum0` filtre sur nos app-ids → `CLASSIFY_DROP` avec
horodatage (FILETIME→unix), exe, `adresse:port`, protocole. Exemple brut :
```
DROP proto=6 vers 1.1.1.1:443
DROP proto=6 vers 9.9.9.9:443
DROP proto=17 vers 8.8.8.8:53
```
C'est la table `audit_sceau` du produit. `FWPM_ENGINE_COLLECT_NET_EVENTS`
etait deja actif systeme → notre `SetOption` renvoie `0x8032000B` (non
bloquant, la collecte tournait) ; ne pas dependre du set.

### Hygiene
Session WFP **dynamique** → `FwpmEngineClose0` detruit sous-couche + filtres :
apres levee, connectivite restauree, **aucun filtre orphelin** (verifie par
les sondes post-levee). Un crash de Waly ne casse jamais le reseau de la
machine.

## Reste a prototyper (chantier 0, hors gate)
- Voie 1 (ACL one-time) : mesurer le droit sur l'objet LAYER pour un standard.
- Sceller les VRAIS process (`flm.exe` + `waly-voice.exe` en appel) et
  verifier que le tour voix loopback survit — integration, pas gate.

## VERDICTS — Chantier C « Huis clos universel » VERTS (2026-09-16)

Sceller n'importe quel agent local (plan `docs/PLAN-2026-09-16-C-huis-clos-
universel.md`, ADR `docs/ADR-2026-09-16-sceller-un-tiers-est-admin.md`). Le
service est passe en version C (garde admin + scelle d'agent tiers).

Procedure : `install-c-eleve.ps1` (UAC, remplace le service par la version C)
puis `run-banc-c-eleve.ps1` (UAC, sequence complete → `banc-c.log`). Client CLI
sous `waly-seal-svc-c.exe`. Agent temoin = copie de `curl.exe` a un chemin
distinct (app-id WFP distinct — on ne scelle QUE ce chemin).

- **Garde ADMIN ✅** : `seal <agent-tiers>` depuis une console STANDARD =
  **REFUSE** (« scelle d'un agent tiers refuse : elevation requise »). Ferme le
  trou latent R6a ou `Sceller {exes:[arbitraire]}` etait accepte de tout
  utilisateur authentifie. Detection : `ImpersonateNamedPipeClient` +
  `TokenElevation` cote serveur (apres lecture de la requete — l'impersonation
  echoue sur pipe octet tant que le client n'a rien ecrit).
- **Scelle d'un agent TIERS ✅** (console elevee) : agent temoin sortait vers
  1.1.1.1:443 AVANT (exit 0) ; APRES scelle = **BLOQUEE** (curl exit 7).
- **Isolation par exe ✅** : le vrai `curl.exe` de System32 (meme binaire, autre
  chemin, NON scelle) sort toujours (exit 0) → WFP filtre par chemin d'exe, on
  ne coupe QUE l'agent vise.
- **Journal exact ✅** : `journal <agent>` = `TCP 1.1.1.1:443` horodate, chemin
  d'exe exact (drain des net events CLASSIFY_DROP filtre sur l'app-id).
- **Levee ✅** : `unseal <agent>` → l'agent re-sort (exit 0).
- **Latence ✅** : pose ~58,7 ms, levee ~38,4 ms (< 100 ms, cible R6a).
- **SAC** : le service release C est passe sans reroll ; installe et Running
  (`C:\Program Files\Waly\waly-seal-svc.exe`, AUTO_START, LocalSystem).

Reste de C : UI « Sceller un autre agent » (C2, desktop) ; one-pager etendu ;
verdict terrain Michee sur un agent tiers reel (Hermes) quand dispo.
