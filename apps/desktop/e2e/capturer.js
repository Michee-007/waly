// Capture la fenêtre de l'app (CDP), après une expression facultative.
// Usage : node capturer.js <fichier.png> [port] [expression JS à évaluer avant] [attente ms]
import { writeFileSync } from 'node:fs';
const [fichier, port = '9222', expr = '', attente = '1200'] = process.argv.slice(2);
if (!fichier) { console.error('usage : node capturer.js <fichier.png> [port] [expression] [attente ms]'); process.exit(2); }
const pages = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
const page = pages.find((p) => p.type === 'page');
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((res, rej) => { ws.onopen = res; ws.onerror = rej; });
let id = 1;
const cdp = (method, params) => new Promise((resolve, reject) => {
  const me = id++;
  const onMsg = (e) => { const m = JSON.parse(e.data); if (m.id !== me) return; ws.removeEventListener('message', onMsg); m.error ? reject(new Error(JSON.stringify(m.error))) : resolve(m.result); };
  ws.addEventListener('message', onMsg);
  ws.send(JSON.stringify({ id: me, method, params: params || {} }));
});
await cdp('Emulation.setDeviceMetricsOverride', { width: 1500, height: 1000, deviceScaleFactor: 1, mobile: false });
if (expr) {
  const r = await cdp('Runtime.evaluate', { expression: expr, returnByValue: true, awaitPromise: true });
  if (r.exceptionDetails) console.error('expression :', r.exceptionDetails.exception?.description);
  else if (r.result?.value !== undefined) console.log(JSON.stringify(r.result.value));
}
await new Promise((r) => setTimeout(r, Number(attente)));
const r = await cdp('Page.captureScreenshot', { format: 'png' });
writeFileSync(fichier, Buffer.from(r.data, 'base64'));
await cdp('Emulation.clearDeviceMetricsOverride');
ws.close();
console.log('capture :', fichier);
