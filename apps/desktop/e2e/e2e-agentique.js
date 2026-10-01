// E2E ch. 3 : sessions-agents (objectif -> travail outille, etapes inline,
// etat) et HITL en ligne (carte d'attente -> Approuver -> l'agent reprend).
// Prerequis : app avec --remote-debugging-port=9222, WALY_DB jetable,
// WALY_DEMO_SENSIBLE=1 (outil sensible de demo), FLM sur 42626.
const PORT = 9222;
async function evalIn(ws, id, expr) {
  return new Promise((resolve, reject) => {
    const onMsg = (ev) => {
      const m = JSON.parse(ev.data);
      if (m.id === id) { ws.removeEventListener('message', onMsg);
        m.error ? reject(new Error(JSON.stringify(m.error))) : resolve(m.result?.result?.value); }
    };
    ws.addEventListener('message', onMsg);
    ws.send(JSON.stringify({ id, method: 'Runtime.evaluate', params: { expression: expr, returnByValue: true, awaitPromise: true } }));
    setTimeout(() => reject(new Error('eval timeout')), 15000);
  });
}
const sleep = (ms) => new Promise(r => setTimeout(r, ms));
let failures = 0;
function check(label, ok, detail) {
  console.log((ok ? 'OK  ' : 'FAIL') + ' ' + label + (detail ? ' — ' + detail : ''));
  if (!ok) failures++;
}

(async () => {
  const pages = await (await fetch(`http://127.0.0.1:${PORT}/json`)).json();
  const page = pages.find(p => p.type === 'page' && /index\.html|Waly/i.test(p.title + p.url));
  const ws = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((res, rej) => { ws.onopen = res; ws.onerror = rej; });
  let id = 1;
  const E = (expr) => evalIn(ws, id++, expr);
  const waitIdle = async (max) => { for (let i=0;i<(max||240);i++){ await sleep(500); if (await E(`document.getElementById('send').textContent`) !== '◼') return true; } return false; };
  const send = async (t) => { await E(`document.getElementById('input').value=${JSON.stringify(t)};document.getElementById('send').click()`); };

  // 1. Nouvelle mission (espace UNIQUE, fusion 2026-09-03) : etat vide
  //    honnete + invite d'objectif dans le composer.
  await sleep(400);
  await E(`document.getElementById('newmission').click()`);
  await sleep(500);
  check('mission : invite a donner un objectif', await E(`!!document.querySelector('.empty')`) === true);
  check('titre Nouvelle mission', await E(`document.getElementById('mtitle').textContent`) === 'Nouvelle mission');
  check('placeholder objectif', /objectif/i.test(await E(`document.getElementById('input').placeholder`)));

  // 2. Objectif sur outils surs -> travail, etapes, bilan, etat fini
  await send('Crée une note titrée Plan démo avec le contenu : tester Waly. Puis crée une tâche titrée Préparer la démo, priorité haute.');
  check('agent termine dans le budget', await waitIdle());
  const steps = await E(`document.querySelectorAll('.step').length`);
  check('etapes outillees inline', steps >= 2, steps + ' etapes');
  const dot = await E(`(document.querySelector('.conv .d')||{}).className`);
  check('etat de session affiche', /fini|attente/.test(dot || ''), dot);

  // 3. Les artefacts sont REELS apres le travail
  await E(`document.getElementById('artifacts').click()`);
  await sleep(500);
  const arts = await E(`document.querySelectorAll('#cbody .crow').length`);
  check('artefacts crees par l agent', arts >= 2, arts + ' entrees');
  await E(`document.getElementById('cclose').click()`);

  // 4. Objectif sensible (nouvelle MISSION) -> carte HITL + etat attente
  await E(`document.getElementById('newmission').click()`);
  await sleep(300);
  await send('Envoie un message à Paul pour lui dire bonjour de ma part.');
  check('tour sensible termine', await waitIdle());
  const waits = await E(`document.querySelectorAll('.wait').length`);
  check('carte d attente HITL', waits >= 1, waits + ' carte(s)');

  // 5. Approuver -> resolution directe + l'agent reprend et conclut
  if (waits >= 1) {
    const before = await E(`document.querySelectorAll('.msg').length`);
    await E(`document.querySelector('.wait .ok').click()`);
    check('continuation terminee', await waitIdle());
    check('carte resolue disparue', await E(`document.querySelectorAll('.wait').length`) === 0);
    const after = await E(`document.querySelectorAll('.msg').length`);
    check('l agent a repris et conclu', after > before, before + ' -> ' + after + ' messages');
    const dot2 = await E(`(document.querySelector('.conv.on .d')||document.querySelector('.conv .d')||{}).className`);
    check('etat final fini', /fini/.test(dot2 || ''), dot2);
  }

  // 6. Espace UNIQUE : conversations ET missions dans la meme liste, les
  //    missions marquees d'un losange.
  await sleep(600);
  const convs = await E(`JSON.stringify([...document.querySelectorAll('.conv .t')].map(e=>e.textContent))`);
  const noms = JSON.parse(convs);
  check('liste unique : fil principal present', noms.some(t=>t.includes('Fil principal')), convs);
  check('liste unique : mission marquee ◇', noms.some(t=>t.startsWith('◇ ')), convs);

  console.log(failures === 0 ? 'TOUT VERT' : failures + ' echec(s)');
  process.exit(failures === 0 ? 0 : 1);
})().catch(e => { console.log('ERREUR:', e.message); process.exit(1); });
