// Refait la capture « Partage » du README : deux instances de la vraie app
// (Alice : CDP 9222, celle qu'on capture ; Bob : CDP 9223) et un relais local.
// Bob envoie une conversation à Alice ; on capture, chez Alice, la
// conversation reçue en attente de son accord.
// Usage : node captures-partage.js <fichier.png> [relais, défaut http://127.0.0.1:18787]
import { writeFileSync } from 'node:fs';

const FICHIER = process.argv[2];
const RELAIS = process.argv[3] || 'http://127.0.0.1:18787';
if (!FICHIER) { console.error('usage : node captures-partage.js <fichier.png> [relais]'); process.exit(2); }
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function instance(port) {
  const pages = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
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
  const invoke = async (cmd, args) => JSON.parse(await ev(`window.__TAURI__.core.invoke(${JSON.stringify(cmd)}, ${JSON.stringify(args || {})}).then(r=>JSON.stringify(r===undefined?null:r)).catch(e=>JSON.stringify({__err:String(e)}))`));
  return { ws, cdp, ev, invoke };
}
const attendre = async (cond, maxS) => { for (let i = 0; i < maxS * 2; i++) { if (await cond()) return true; await sleep(500); } return false; };
const fermerDemo = (X) => X.ev(`(function(){ const d=document.getElementById('demo'); if(d && !d.hidden){ const b=document.getElementById('demook'); if(b) b.click(); } return 1; })()`);

const A = await instance(9222);
let B = await instance(9223);
await fermerDemo(A); await fermerDemo(B); await sleep(2500);
for (const X of [A, B]) for (const c of (await X.invoke('core_partage')).contacts) await X.invoke('core_contact_retirer', { id: c.id });
await A.invoke('core_partage_regler', { relais: RELAIS, nom: 'Alice' });
await B.invoke('core_partage_regler', { relais: RELAIS, nom: 'Bob' });
const a = await A.invoke('core_partage'), b = await B.invoke('core_partage');
console.log('Alice ajoute Bob :', await A.invoke('core_contact_ajouter', { nom: 'Bob', code: b.code }));
const idAlice = await B.invoke('core_contact_ajouter', { nom: 'Alice', code: a.code });
console.log('Bob ajoute Alice :', idAlice);

// Une conversation chez Bob (un vrai tour du modèle local).
await B.invoke('core_new_session'); await B.ev('location.reload()'); await sleep(3000); B.ws.close();
B = await instance(9223);
await B.ev(`(function(){ const i=document.getElementById('input'); i.value='Propose trois titres courts pour un article sur les assistants IA qui tournent en local.'; i.dispatchEvent(new Event('input')); document.getElementById('send').click(); return 1; })()`);
await attendre(async () => (await B.invoke('core_history'))[1].length >= 2 && await B.ev(`document.getElementById('send').title!=='Arrêter'`), 240);
console.log('conversation de Bob :', (await B.invoke('core_history'))[1].length, 'messages');

let envoi = null;
await attendre(async () => { envoi = await B.invoke('core_partage_envoyer', { contact: idAlice, titre: 'Titres pour l’article' }); return !envoi.__err; }, 60);
console.log('envoi :', JSON.stringify(envoi));
const recu = await attendre(async () => (await A.invoke('core_partage')).recus.length >= 1, 90);
console.log('reçu chez Alice :', recu, JSON.stringify((await A.invoke('core_partage')).recus));

await A.cdp('Emulation.setDeviceMetricsOverride', { width: 1500, height: 1000, deviceScaleFactor: 1, mobile: false });
await A.ev(`(function(){ const m=document.getElementById('me'); if(!document.querySelector('[data-sp]')) m.click(); return 1; })()`);
await sleep(800);
await A.ev(`(function(){ const x=document.querySelector('[data-sp="partage"]'); if(x) x.click(); return 1; })()`);
await sleep(2500);
const r = await A.cdp('Page.captureScreenshot', { format: 'png' });
writeFileSync(FICHIER, Buffer.from(r.data, 'base64'));
console.log('capture :', FICHIER);
await A.cdp('Emulation.clearDeviceMetricsOverride');
A.ws.close(); B.ws.close();
process.exit(recu ? 0 : 1);
