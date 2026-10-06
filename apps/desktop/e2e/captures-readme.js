// Refait deux captures du README sur la vraie app (CDP) : la réflexion
// approfondie (un vrai tour du modèle local) et le catalogue de modèles.
// Usage : node captures-readme.js <dossier des captures> [port CDP]
// Prérequis : app lancée avec le port de pilotage, moteur local qui répond.
import { writeFileSync } from 'node:fs';
import { join } from 'node:path';

const SORTIE = process.argv[2] || '.';
const PORT = Number(process.argv[3] || 9222);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function connect() {
  const pages = await (await fetch(`http://127.0.0.1:${PORT}/json`)).json();
  const ws = new WebSocket(pages.find((p) => p.type === 'page').webSocketDebuggerUrl);
  await new Promise((res, rej) => { ws.onopen = res; ws.onerror = rej; });
  let id = 1;
  const cdp = (method, params) => new Promise((resolve, reject) => {
    const me = id++;
    const onMsg = (e) => { const m = JSON.parse(e.data); if (m.id !== me) return; ws.removeEventListener('message', onMsg); m.error ? reject(new Error(JSON.stringify(m.error))) : resolve(m.result); };
    ws.addEventListener('message', onMsg);
    ws.send(JSON.stringify({ id: me, method, params: params || {} }));
  });
  const ev = async (expr) => (await cdp('Runtime.evaluate', { expression: expr, returnByValue: true, awaitPromise: true })).result?.value;
  const invoke = (cmd, args) => ev(`window.__TAURI__.core.invoke(${JSON.stringify(cmd)}, ${JSON.stringify(args || {})}).then(r=>JSON.stringify(r===undefined?null:r)).catch(e=>JSON.stringify({__err:String(e)}))`);
  await cdp('Emulation.setDeviceMetricsOverride', { width: 1500, height: 1000, deviceScaleFactor: 1, mobile: false });
  const capture = async (nom) => {
    await sleep(900);
    const r = await cdp('Page.captureScreenshot', { format: 'png' });
    writeFileSync(join(SORTIE, nom), Buffer.from(r.data, 'base64'));
    console.log('capture :', join(SORTIE, nom));
  };
  return { ws, cdp, ev, invoke, capture };
}

let c = await connect();
// La démonstration du premier lancement couvre la fenêtre : on la ferme.
await c.ev(`(function(){ const d=document.getElementById('demo'); if(d && !d.hidden){ const b=document.getElementById('demook'); if(b) b.click(); } return 1; })()`);
await sleep(2500);

// 1. Réflexion approfondie : une conversation neuve, une question à étapes.
await c.invoke('core_new_session');
await c.ev('location.reload()'); await sleep(3000); c.ws.close();
c = await connect();
await c.invoke('core_reglage_set', { cle: 'reflexion', valeur: 'oui' });
const question = "J'ai 3 réunions de 45 minutes avec 10 minutes de pause entre chaque. La première commence à 9h20. À quelle heure finit la dernière ?";
await c.ev(`(function(){ const i=document.getElementById('input'); i.value=${JSON.stringify(question)}; i.dispatchEvent(new Event('input')); document.getElementById('send').click(); return 1; })()`);
let fini = false;
for (let k = 0; k < 600 && !fini; k++) {
  await sleep(500);
  fini = k > 3 && await c.ev(`document.getElementById('send').title!=='Arrêter'`);
}
const etat = JSON.parse(await c.ev(`JSON.stringify((function(){ const w=[...document.querySelectorAll('#col .waly')]; const l=w[w.length-1]; const t=l&&l.querySelector('.think');
  if(t) t.open=true; return { reflexion: t ? t.querySelector('.tt').textContent.length : 0, reponse: l ? l.querySelector('.body').textContent : '' }; })())`));
console.log('réflexion :', etat.reflexion, 'caractères ; réponse :', etat.reponse);
await c.capture('reflexion.png');
await c.invoke('core_reglage_set', { cle: 'reflexion', valeur: 'non' });

// 2. Le catalogue de modèles (Paramètres › Modèles).
await c.ev(`(function(){ document.getElementById('me').click(); return 1; })()`);
await sleep(800);
await c.ev(`(function(){ const b=document.querySelector('[data-sp="modeles"]'); if(b) b.click(); return 1; })()`);
await sleep(4000);
await c.capture('modeles.png');

await c.cdp('Emulation.clearDeviceMetricsOverride');
c.ws.close();
