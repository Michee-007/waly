// Refait la capture « appel vidéo » du README sur la vraie app (CDP) : lance
// un appel à voix haute, passe en vidéo, attend que Waly voie quelqu'un,
// capture, raccroche. ALLUME LA CAMÉRA ET LE MICRO une vingtaine de secondes.
// Usage : node captures-appel-video.js <fichier.png> [port CDP]
import { writeFileSync } from 'node:fs';

const FICHIER = process.argv[2];
const PORT = Number(process.argv[3] || 9222);
if (!FICHIER) { console.error('usage : node captures-appel-video.js <fichier.png> [port]'); process.exit(2); }
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

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
const etat = () => ev(`JSON.stringify({ page: !document.getElementById('callpage').hidden, etat: document.getElementById('callstate').textContent, titre: document.getElementById('calltitle').textContent, vue: document.getElementById('selfview').classList.contains('on'), puce: document.getElementById('callchip').textContent })`).then(JSON.parse);

await cdp('Emulation.setDeviceMetricsOverride', { width: 1500, height: 1000, deviceScaleFactor: 1, mobile: false });
let ok = false;
try {
  await ev(`document.getElementById('micbtn').click()`);
  for (let i = 0; i < 40 && !(await etat()).page; i++) await sleep(500);
  console.log('appel :', JSON.stringify(await etat()));
  await ev(`document.getElementById('ccam').click()`);
  let e = null;
  for (let i = 0; i < 60; i++) { await sleep(500); e = await etat(); if (e.vue && /voit/.test(e.etat)) break; }
  console.log('vidéo :', JSON.stringify(e));
  ok = e.vue;
  await sleep(1500);
  const r = await cdp('Page.captureScreenshot', { format: 'png' });
  writeFileSync(FICHIER, Buffer.from(r.data, 'base64'));
  console.log('capture :', FICHIER);
} finally {
  // Quoi qu'il arrive, on raccroche : caméra et micro s'éteignent.
  await ev(`document.getElementById('hangup').click()`);
  await sleep(2500);
  console.log('raccroché :', JSON.stringify(await etat()));
  await cdp('Emulation.clearDeviceMetricsOverride');
  ws.close();
}
process.exit(ok ? 0 : 1);
