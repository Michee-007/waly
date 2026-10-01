// E2E mode Écran (R5 ch. 3) : ouvrir le partage d'écran (charge l'OCR sous
// SAC in-app) → « lis-moi ce qui est à l'écran » (capture + OCR + tour) →
// réponse non vide → fermer. Prérequis : app avec --remote-debugging-port=9222,
// FLM qwen3vl-it:4b sur 42626, base WALY_DB jetable.

const CDP = 'http://127.0.0.1:9222';

async function ws() {
  const list = await (await fetch(CDP + '/json')).json();
  const page = list.find(t => t.type === 'page');
  if (!page) throw new Error('aucune page CDP');
  return new WebSocket(page.webSocketDebuggerUrl);
}

let id = 0;
function evalJs(sock, expr) {
  return new Promise((resolve, reject) => {
    const mid = ++id;
    const onmsg = (e) => {
      const m = JSON.parse(e.data);
      if (m.id !== mid) return;
      sock.removeEventListener('message', onmsg);
      if (m.error) return reject(new Error(JSON.stringify(m.error)));
      resolve(m.result.result ? m.result.result.value : undefined);
    };
    sock.addEventListener('message', onmsg);
    sock.send(JSON.stringify({
      id: mid, method: 'Runtime.evaluate',
      params: { expression: expr, returnByValue: true, awaitPromise: true },
    }));
  });
}
const sleep = (ms) => new Promise(r => setTimeout(r, ms));

(async () => {
  const sock = await ws();
  await new Promise(r => sock.addEventListener('open', r));

  // 1. Le bouton ▣ Écran existe.
  const btn = await evalJs(sock, `!!document.getElementById('ecran')`);
  if (!btn) throw new Error('bouton ▣ Écran absent');

  // 2. Ouvrir le partage d'écran → charge l'OCR (sous SAC, in-app).
  await evalJs(sock, `document.getElementById('ecran').click()`);
  for (let k = 0; k < 15; k++) {
    await sleep(500);
    if (await evalJs(sock, `document.getElementById('ecran').textContent`) !== '▣ …') break;
  }
  const label = await evalJs(sock, `document.getElementById('ecran').textContent`);
  console.log('mode écran →', label);
  if (!label.includes('actif')) throw new Error('le partage d’écran ne s’est pas activé (OCR chargé ?) : ' + label);

  // 3. « Lis-moi ce qui est à l'écran » → capture + OCR + tour end-to-end.
  const t0 = Date.now();
  await evalJs(sock, `
    (async () => {
      const i = document.getElementById('input');
      i.value = "Lis-moi ce qui est affiché à l'écran.";
      i.dispatchEvent(new Event('input'));
      document.getElementById('send').click();
    })()`);
  let fini = false;
  for (let k = 0; k < 40; k++) {
    await sleep(1000);
    if (await evalJs(sock, `document.getElementById('send').textContent`) === '◉') { fini = true; break; }
  }
  if (!fini) throw new Error('tour écran jamais fini (40 s)');
  const secs = ((Date.now() - t0) / 1000).toFixed(1);
  const reponse = await evalJs(sock,
    `([...document.querySelectorAll('.msg:not(.you) p')].pop() || {}).textContent || ''`);
  console.log(`tour écran ${secs} s | réponse: ${reponse ? reponse.slice(0, 160) : '(VIDE)'}`);
  if (!reponse || reponse.length < 3) throw new Error('réponse vide au tour écran');

  // 4. Fermer le partage.
  await evalJs(sock, `document.getElementById('ecran').click()`);
  await sleep(500);
  const off = await evalJs(sock, `document.getElementById('ecran').textContent`);
  console.log('fermeture →', off);

  console.log('\nE2E ÉCRAN : VERT ✅');
  sock.close();
  process.exit(0);
})().catch(e => { console.error('E2E ÉCRAN ÉCHEC:', e.message); process.exit(1); });
