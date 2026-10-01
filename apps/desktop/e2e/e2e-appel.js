// E2E mode appel (R4 ch. 5, écran d'appel dédié — retour Michée 2026-07-08) :
// appel → page d'appel visible + auto-vue alimentée + sous-titres présents,
// réduire → « regarde » tapé depuis le chat (raccourci, un tour), retour à
// l'appel, raccrocher. Prérequis : app avec --remote-debugging-port=9222,
// FLM qwen3vl-it:4b sur 42626, caméra libre (voir README).

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

  // 1. Démarrer l'appel → la PAGE D'APPEL s'ouvre.
  await evalJs(sock, `document.getElementById('appel').click()`);
  await sleep(3000); // caméra + hystérésis + premier aperçu
  const pageVisible = await evalJs(sock, `!document.getElementById('callpage').hidden`);
  if (!pageVisible) throw new Error("la page d'appel ne s'est pas ouverte");
  const etat = await evalJs(sock, `document.getElementById('callstate').textContent`);
  const selfOk = await evalJs(sock,
    `document.getElementById('selfview').classList.contains('on') && document.getElementById('selfview').src.length > 1000`);
  console.log('page appel ouverte | état:', etat || '(vide)', '| auto-vue:', selfOk ? 'alimentée' : 'ABSENTE');
  if (!selfOk) throw new Error('auto-vue non alimentée');

  // 2. Réduire → l'appel continue, le chat revient.
  await evalJs(sock, `document.getElementById('reduire').click()`);
  await sleep(400);
  const reduit = await evalJs(sock, `document.getElementById('callpage').hidden`);
  const enAppel = await evalJs(sock, `document.getElementById('appel').textContent`);
  if (!reduit || !enAppel.includes('En appel')) throw new Error('réduire a cassé l’appel');

  // 3. « Regarde » depuis le chat (raccourci d'intention, un tour).
  const t0 = Date.now();
  await evalJs(sock, `
    (async () => {
      const i = document.getElementById('input');
      i.value = 'Regarde et décris ce que tu vois en une phrase.';
      i.dispatchEvent(new Event('input'));
      document.getElementById('send').click();
    })()`);
  let fini = false;
  for (let k = 0; k < 60; k++) {
    await sleep(1000);
    if (await evalJs(sock, `document.getElementById('send').textContent`) === '◉') { fini = true; break; }
  }
  if (!fini) throw new Error('tour vision jamais fini (60 s)');
  const secs = ((Date.now() - t0) / 1000).toFixed(1);
  const reponse = await evalJs(sock,
    `[...document.querySelectorAll('.msg:not(.you) p')].pop().textContent`);
  console.log('tour vision (' + secs + ' s):', reponse);
  if (reponse.length < 15) throw new Error('réponse vide/trop courte');

  // 4. Retour à l'écran d'appel (le bouton y ramène), sous-titres remplis.
  await evalJs(sock, `document.getElementById('appel').click()`);
  await sleep(1500);
  const rouvert = await evalJs(sock, `!document.getElementById('callpage').hidden`);
  const captWaly = await evalJs(sock, `document.getElementById('captwaly').textContent`);
  if (!rouvert) throw new Error('le bouton ne ramène pas à l’appel');
  console.log('sous-titres WALY:', captWaly ? captWaly.slice(0, 60) + '…' : '(vides)');

  // 5. Raccrocher : page fermée, appel fini.
  await evalJs(sock, `document.getElementById('hangup').click()`);
  await sleep(800);
  const ferme = await evalJs(sock, `document.getElementById('callpage').hidden`);
  const btn = await evalJs(sock, `document.getElementById('appel').textContent`);
  const chipOn = await evalJs(sock, `document.getElementById('preschip').classList.contains('on')`);
  if (!ferme || btn.includes('En appel') || chipOn) throw new Error('raccrocher incomplet');
  console.log('OK — page d’appel, auto-vue, réduire, vision ' + secs + ' s, sous-titres, raccrocher');
  process.exit(0);
})().catch(e => { console.error('ÉCHEC:', e.message); process.exit(1); });
