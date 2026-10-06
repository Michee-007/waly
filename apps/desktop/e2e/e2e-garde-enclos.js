// E2E « la Garde, étape 4 : l'enclos » (2026-10-06) via CDP, sur la VRAIE
// app et un VRAI agent qui tourne. Le script passe par l'interface : il ouvre
// la Garde, choisit l'agent par son programme, clique « Mettre dans
// l'enclos », attend qu'il y tourne, donne un dossier, refait l'essai, puis
// le sort de l'enclos. Il prend des captures en route.
//
// Usage : node e2e-garde-enclos.js "<exe de l'agent>" "<dossier à donner>" [dossier des captures] [port CDP]
// Prérequis : l'app lancée avec WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=
// --remote-debugging-port=9222, l'agent en cours d'exécution, le compte de
// l'enclos déjà créé (sinon Windows demande l'accord pendant l'essai).
// Le choix du dossier se fait par la commande que l'interface appelle après
// le sélecteur de Windows : un dialogue natif ne se pilote pas par CDP.
import { writeFileSync } from 'node:fs';
import { join } from 'node:path';

const EXE = process.argv[2];
const DOSSIER = process.argv[3];
const SORTIE = process.argv[4] || '.';
const PORT = Number(process.argv[5] || 9222);
const SURVEILLER = process.env.WALY_E2E_SURVEILLER === '1';
if (!EXE || !DOSSIER) { console.error('usage : node e2e-garde-enclos.js <exe> <dossier> [captures] [port]'); process.exit(2); }
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function connect() {
  const pages = await (await fetch(`http://127.0.0.1:${PORT}/json`)).json();
  const page = pages.find((p) => p.type === 'page');
  if (!page) throw new Error('page Waly introuvable');
  const ws = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((res, rej) => { ws.onopen = res; ws.onerror = rej; });
  let id = 1;
  const cdp = (method, params) => new Promise((resolve, reject) => {
    const me = id++;
    const onMsg = (e) => {
      const m = JSON.parse(e.data);
      if (m.id !== me) return;
      ws.removeEventListener('message', onMsg);
      m.error ? reject(new Error(JSON.stringify(m.error))) : resolve(m.result);
    };
    ws.addEventListener('message', onMsg);
    ws.send(JSON.stringify({ id: me, method, params: params || {} }));
  });
  const ev = async (expr) => {
    const r = await cdp('Runtime.evaluate', { expression: expr, returnByValue: true, awaitPromise: true });
    if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description || JSON.stringify(r.exceptionDetails));
    return r.result?.value;
  };
  const invoke = async (cmd, args) => JSON.parse(await ev(`window.__TAURI__.core.invoke(${JSON.stringify(cmd)}, ${JSON.stringify(args || {})}).then(r=>JSON.stringify(r===undefined?null:r)).catch(e=>JSON.stringify({__err:String(e)}))`));
  return { ws, cdp, ev, invoke };
}

let echecs = 0;
const dire = (ok, quoi, detail) => { if (!ok) echecs++; console.log(`${ok ? 'OK  ' : 'ECHEC'} ${quoi}${detail ? ' — ' + detail : ''}`); };

const { ws, cdp, ev, invoke } = await connect();
// La page Garde est-elle à l'écran ? Sinon on la rouvre (la démonstration du
// premier lancement la referme en finissant).
const garder = async () => {
  const vue = await ev(`(function(){ const c=document.getElementById('gdcarte'); return !!c && c.offsetParent!==null; })()`);
  if (!vue) { await ev(`document.getElementById('gardebtn').click()`); await sleep(1500); }
};
const capture = async (nom, garde) => {
  if (garde) { await garder(); await choisir(); }
  await sleep(900);
  const r = await cdp('Page.captureScreenshot', { format: 'png' });
  const f = join(SORTIE, nom);
  writeFileSync(f, Buffer.from(r.data, 'base64'));
  console.log(`     capture : ${f}`);
};
const clic = (sel) => ev(`(function(){ const e=document.querySelector(${JSON.stringify(sel)}); if(!e) return false; e.click(); return true; })()`);
const texte = (sel) => ev(`(function(){ const e=document.querySelector(${JSON.stringify(sel)}); return e ? e.innerText : null; })()`);
const agent = async () => (await invoke('core_garde')).agents.find((a) => a.exe.toLowerCase() === EXE.toLowerCase());
// Choisit l'agent sur le graphe. Plusieurs agents peuvent porter le même nom
// (trois « Claude » sur la machine de référence, dont la session qui lance
// cet essai) : on clique les noeuds un à un jusqu'à ce que le panneau montre
// CE programme. Choisir un noeud ne change rien.
// Le programme que le panneau montre (chemin complet, porté par l'élément).
const programme = () => ev(`(function(){ const e=document.querySelector('#gdcote [data-exe]'); return e ? e.dataset.exe : ''; })()`);
const sansBlancs = (t) => (t || '').replace(/\s+/g, '').toLowerCase();
const choisir = async () => {
  const n = await ev(`document.querySelectorAll('#gdcarte .noeud').length`);
  for (let i = 1; i < n; i++) {
    await ev(`(function(){ const n=document.querySelectorAll('#gdcarte .noeud')[${i}]; if(n) n.dispatchEvent(new MouseEvent('click',{bubbles:true})); return 1; })()`);
    await sleep(450);
    if (sansBlancs(await programme()) === sansBlancs(EXE)) return true;
  }
  return false;
};
const attendre = async (cond, maxS) => { for (let i = 0; i < maxS * 2; i++) { const v = await cond(); if (v) return v; await sleep(500); } return null; };

await cdp('Emulation.setDeviceMetricsOverride', { width: 1500, height: 1000, deviceScaleFactor: 1, mobile: false });
// Premier lancement sur une base neuve : la démonstration du scellé couvre
// la fenêtre. On la laisse finir, on la capture, on la ferme.
if (await ev(`(function(){ const d=document.getElementById('demo'); return !!d && !d.hidden; })()`)) {
  await attendre(() => ev(`(function(){ const b=document.getElementById('demook'); return !!b && !b.hidden && b.offsetParent!==null; })()`), 20);
  await capture('preuve.png');
  await clic('#demook');
  await sleep(2500);
}
await clic('#gardebtn');
dire(!!(await attendre(() => texte('#gdphrase').then((t) => t && t !== '…' ? t : null), 15)), 'la Garde s’ouvre', await texte('#gdphrase'));

// 1. L'agent tourne, hors de l'enclos.
let a = await agent();
dire(!!a && a.en_cours && !a.enclos, 'l’agent tourne sous ton compte', a ? `${a.nom} — ${a.exe}` : 'introuvable : lance-le d’abord');
if (!a) { ws.close(); process.exit(1); }
dire(await choisir(), 'son noeud est choisi sur le graphe');
const panneau = await texte('#gdcote');
// Garde-fou : on ne clique que si le panneau montre bien CE programme.
if (sansBlancs(await programme()) !== sansBlancs(EXE)) {
  dire(false, 'le panneau montre le bon programme', panneau); ws.close(); process.exit(1);
}
await capture('garde-avant-enclos.png', true);

// 2. Le mettre dans l'enclos, par le bouton et son dialogue.
dire(await clic('#gdenclos'), 'bouton « Mettre dans l’enclos »');
await sleep(400);
console.log('     dialogue : ' + (await texte('#dlgtitle')) + ' | ' + (await texte('#dlgtext')));
await clic('#dlgok');
a = await attendre(async () => { const x = await agent(); return x && x.enclos && x.en_cours ? x : null; }, 120);
dire(!!a, 'l’agent tourne DANS l’enclos', await texte('#gdvoix'));
if (!a) { ws.close(); process.exit(1); }
await choisir();
let g = await invoke('core_garde');
dire(g.enclos.pret && g.enclos.profil_invisible === true, 'ton dossier personnel lui est fermé (essai)', `essai du ${g.enclos.profil_essaye}`);
for (const d of g.enclos.dossiers) console.log(`     ${d.droit.padEnd(8)} ${d.chemin} [lit=${d.lit} ecrit=${d.ecrit} conforme=${d.conforme}] ${d.pourquoi}`);
dire(g.enclos.dossiers.every((d) => d.conforme === true), 'chaque réglage est confirmé par sa sonde');

// 3. Donner un dossier en écriture, puis refaire tous les essais par le bouton.
let r = await invoke('core_enclos_dossier', { chemin: DOSSIER, droit: 'ecriture' });
dire(r.ok && r.conforme === true, 'un dossier donné en écriture, confirmé par la sonde', JSON.stringify(r));
await sleep(3200); // la page se relit toutes les 2,5 s
await choisir();
await clic('#gdees');
const voix = await attendre(() => texte('#gdvoix').then((t) => t && /Essais à l’instant|⚠/.test(t) ? t : null), 40);
dire(!!voix && !voix.includes('⚠'), 'bouton « Refaire l’essai »', voix);
await choisir();
await capture('garde.png', true);

// 4. Le repasser en lecture seule par la ligne du panneau n'existe pas : on
// reprend le dossier en cliquant sa ligne, comme le ferait l'utilisateur.
g = await invoke('core_garde');
const i = g.enclos.dossiers.findIndex((d) => d.chemin.toLowerCase() === DOSSIER.toLowerCase());
await ev(`(function(){ const b=document.querySelector('#gdcote .gd-acces [data-i="${i}"]'); if(b) b.click(); return 1; })()`);
const repris = await attendre(async () => !(await invoke('core_garde')).enclos.dossiers.some((d) => d.chemin.toLowerCase() === DOSSIER.toLowerCase()), 15);
dire(!!repris, 'le dossier est repris en cliquant sa ligne', await texte('#gdvoix'));

// 5. (facultatif) la surveillance de cet agent, dans le service installé :
// Windows demande l'accord.
if (SURVEILLER) {
  await choisir();
  await clic('#gdsurv');
  const s = await attendre(async () => { const x = await agent(); return x && x.surveille ? x : null; }, 60);
  dire(!!s, 'la surveillance de l’agent est allumée', await texte('#gdvoix'));
  if (s) {
    await sleep(8000);
    const x = await agent();
    console.log(`     vu en 8 s : ${x.vu_fichiers} fichier(s), ${x.vu_programmes} programme(s), ${x.vu_internet} adresse(s)`);
    await choisir(); await capture('garde-enclos-surveille.png', true);
    await invoke('core_regard', { mode: 'rien', exes: [] });
  }
}

// 6. Le sortir de l'enclos : il s'arrête.
await choisir();
await clic('#gdesor'); await sleep(400); await clic('#dlgok');
const sorti = await attendre(async () => { const x = await agent(); return !x || (!x.enclos && !x.en_cours) ? true : null; }, 20);
dire(!!sorti, 'sorti de l’enclos et arrêté', await texte('#gdvoix'));

await cdp('Emulation.clearDeviceMetricsOverride');
ws.close();
console.log(echecs ? `\n${echecs} échec(s)` : '\nTout est passé.');
process.exit(echecs ? 1 : 0);
