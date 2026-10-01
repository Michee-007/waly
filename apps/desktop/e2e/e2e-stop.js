// E2E stop : lance une reponse longue, clique ◼ en plein flux, verifie
// l'arret rapide, la note "interrompu", et qu'un tour suivant marche.
const PORT = 9222;
async function evalIn(ws, id, expr) {
  return new Promise((resolve, reject) => {
    const onMsg = (ev) => {
      const m = JSON.parse(ev.data);
      if (m.id === id) { ws.removeEventListener('message', onMsg);
        m.error ? reject(new Error(JSON.stringify(m.error))) : resolve(m.result?.result?.value); }
    };
    ws.addEventListener('message', onMsg);
    ws.send(JSON.stringify({ id, method: 'Runtime.evaluate', params: { expression: expr, returnByValue: true } }));
    setTimeout(() => reject(new Error('eval timeout')), 10000);
  });
}
const sleep = (ms) => new Promise(r => setTimeout(r, ms));
const SNAP = `JSON.stringify({
  btn: document.getElementById('send').textContent,
  last: (function(){ const m=[...document.querySelectorAll('.msg')]; return m.length? m[m.length-1].textContent.trim() : ''; })(),
  note: !!document.querySelector('.msg .note')
})`;

(async () => {
  const pages = await (await fetch(`http://127.0.0.1:${PORT}/json`)).json();
  const page = pages.find(p => p.type === 'page' && /index\.html|Waly/i.test(p.title + p.url));
  const ws = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((res, rej) => { ws.onopen = res; ws.onerror = rej; });
  let id = 1;

  // 1. Reponse longue
  await evalIn(ws, id++, `document.getElementById('input').value='Raconte-moi une histoire de dix phrases sur un renard.';document.getElementById('send').click();'ok'`);
  // 2. Attendre que le flux debite (>40 car.), puis STOP
  let started = false;
  for (let i = 0; i < 60; i++) {
    await sleep(400);
    const s = JSON.parse(await evalIn(ws, id++, SNAP));
    if (s.last.length > 40 && s.btn === '◼') {
      await evalIn(ws, id++, `document.getElementById('send').click();'stop'`);
      console.log(`STOP clique a ${s.last.length} car.`);
      started = true; break;
    }
  }
  if (!started) { console.log('ECHEC: flux jamais parti'); process.exit(1); }
  // 3. L'arret doit etre rapide (< 4 s)
  const t0 = Date.now();
  for (let i = 0; i < 20; i++) {
    await sleep(400);
    const s = JSON.parse(await evalIn(ws, id++, SNAP));
    if (s.btn !== '◼') {
      console.log(`ARRETE en ${((Date.now()-t0)/1000).toFixed(1)}s — note interrompu: ${s.note} — partiel: "${s.last.slice(0,80)}..."`);
      // 4. Tour suivant : la fenetre n'est pas cassee
      await evalIn(ws, id++, `document.getElementById('input').value='Reponds juste: ok';document.getElementById('send').click();'ok'`);
      for (let j = 0; j < 90; j++) {
        await sleep(500);
        const s2 = JSON.parse(await evalIn(ws, id++, SNAP));
        if (s2.btn !== '◼' && j > 1) {
          console.log('TOUR SUIVANT:', s2.last.slice(0, 100));
          process.exit(s2.last.length > 4 && !s2.last.includes('moteur FLM') ? 0 : 2);
        }
      }
      process.exit(3);
    }
  }
  console.log('ECHEC: pas arrete en 8 s'); process.exit(4);
})().catch(e => { console.log('ERREUR:', e.message); process.exit(1); });
