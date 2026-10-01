// E2E du desktop Waly via CDP (WebView2 --remote-debugging-port=9222) :
// tape un message dans l'UI reelle, clique Envoyer, observe le streaming.
const PORT = 9222;

async function evalIn(ws, id, expr) {
  return new Promise((resolve, reject) => {
    const onMsg = (ev) => {
      const m = JSON.parse(ev.data);
      if (m.id === id) {
        ws.removeEventListener('message', onMsg);
        if (m.error) reject(new Error(JSON.stringify(m.error)));
        else resolve(m.result?.result?.value);
      }
    };
    ws.addEventListener('message', onMsg);
    ws.send(JSON.stringify({ id, method: 'Runtime.evaluate', params: { expression: expr, returnByValue: true } }));
    setTimeout(() => reject(new Error('eval timeout')), 10000);
  });
}

const sleep = (ms) => new Promise(r => setTimeout(r, ms));

(async () => {
  const pages = await (await fetch(`http://127.0.0.1:${PORT}/json`)).json();
  const page = pages.find(p => p.type === 'page' && /index\.html|Waly/i.test(p.title + p.url));
  if (!page) { console.log('ECHEC: page Waly introuvable', pages.map(p => p.url)); process.exit(1); }
  const ws = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((res, rej) => { ws.onopen = res; ws.onerror = rej; });

  let id = 1;
  // 1. Envoyer un message par l'UI reelle
  await evalIn(ws, id++, `
    document.getElementById('input').value = 'Reponds en une seule courte phrase : quelle heure est-il ?';
    document.getElementById('send').click(); 'clicked'`);
  console.log('message envoye via l\'UI');

  // 2. Observer : glyphe du bouton (◼ = tour en cours), texte du dernier msg
  let lastLen = -1, growth = [];
  const t0 = Date.now();
  for (let i = 0; i < 240; i++) {
    await sleep(500);
    const snap = await evalIn(ws, id++, `JSON.stringify({
      btn: document.getElementById('send').textContent,
      state: document.getElementById('mstate').textContent,
      last: (function(){ const m=[...document.querySelectorAll('.msg')]; return m.length? m[m.length-1].textContent.trim() : ''; })()
    })`);
    const s = JSON.parse(snap);
    if (s.last.length !== lastLen) { growth.push(`${((Date.now()-t0)/1000).toFixed(1)}s: ${s.last.length} car.` + (s.state?` [${s.state}]`:'')); lastLen = s.last.length; }
    if (s.btn !== '◼' && i > 1) { // bouton redevenu "envoyer" -> tour fini
      console.log('progression du texte:', growth.join(' | '));
      console.log('REPONSE FINALE:', s.last);
      console.log('duree totale:', ((Date.now()-t0)/1000).toFixed(1), 's');
      process.exit(s.last.length > 5 && !s.last.includes('moteur FLM') ? 0 : 2);
    }
  }
  console.log('TIMEOUT apres 120s; progression:', growth.join(' | '));
  process.exit(3);
})().catch(e => { console.log('ERREUR:', e.message); process.exit(1); });
