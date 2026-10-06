// E2E « réflexion approfondie » (lot 3, 2026-10-01) via CDP : active le
// réglage, envoie une question à étapes, observe le bloc « Réflexion » se
// remplir PUIS la réponse, vérifie que la réflexion n'est pas dans la bulle,
// recharge la page (le bloc doit revenir, replié), puis coupe le réglage et
// vérifie qu'un tour normal n'affiche aucune balise.
// Usage : node e2e-reflexion.js [port CDP, défaut 9222]
const PORT = Number(process.argv[2] || 9222);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function connect() {
  const pages = await (await fetch(`http://127.0.0.1:${PORT}/json`)).json();
  const page = pages.find((p) => p.type === 'page' && /index\.html|tauri\.localhost|Waly/i.test(p.title + p.url));
  if (!page) throw new Error('page Waly introuvable: ' + pages.map((p) => p.url).join(', '));
  const ws = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((res, rej) => { ws.onopen = res; ws.onerror = rej; });
  let id = 1;
  const ev = (expr) => new Promise((resolve, reject) => {
    const me = id++;
    const onMsg = (e) => {
      const m = JSON.parse(e.data);
      if (m.id !== me) return;
      ws.removeEventListener('message', onMsg);
      if (m.error || m.result?.exceptionDetails) reject(new Error(JSON.stringify(m.error || m.result.exceptionDetails)));
      else resolve(m.result?.result?.value);
    };
    ws.addEventListener('message', onMsg);
    ws.send(JSON.stringify({ id: me, method: 'Runtime.evaluate', params: { expression: expr, returnByValue: true, awaitPromise: true } }));
  });
  return { ws, ev };
}

const ETAT = `JSON.stringify((function(){
  const w=[...document.querySelectorAll('#col .waly')]; const l=w[w.length-1];
  const t=l&&l.querySelector('.think');
  return { busy: document.getElementById('send').title==='Arrêter', n: w.length,
    think: t ? t.querySelector('.tt').textContent : null, open: t ? t.open : null,
    label: t ? t.querySelector('summary').textContent : null,
    body: l ? l.querySelector('.body').textContent : '', err: l ? l.classList.contains('err') : false };
})())`;

async function tour(ev, message, maxS) {
  await ev(`(function(){ const i=document.getElementById('input'); i.value=${JSON.stringify(message)}; i.dispatchEvent(new Event('input')); document.getElementById('send').click(); return 1; })()`);
  const t0 = Date.now();
  let premierThink = null, premierMot = null, s = null;
  for (let k = 0; k < maxS * 2; k++) {
    await sleep(500);
    s = JSON.parse(await ev(ETAT));
    const t = ((Date.now() - t0) / 1000).toFixed(1);
    if (s.think && premierThink === null) { premierThink = t; console.log(`  ${t}s : la réflexion commence (bloc ouvert=${s.open})`); }
    if (s.body && premierMot === null) { premierMot = t; console.log(`  ${t}s : la réponse commence (bloc ouvert=${s.open}, « ${s.label} »)`); }
    if (!s.busy && k > 1) { console.log(`  ${t}s : tour fini`); break; }
  }
  return { ...s, premierThink, premierMot };
}

(async () => {
  let { ws, ev } = await connect();
  const invoke = (cmd, args) => ev(`window.__TAURI__.core.invoke(${JSON.stringify(cmd)}, ${JSON.stringify(args || {})}).then(r=>JSON.stringify(r))`);
  let ok = true;
  const verifie = (cond, quoi) => { console.log((cond ? 'OK   ' : 'ECHEC ') + quoi); if (!cond) ok = false; };

  await invoke('core_new_session');
  await ev(`location.reload()`); await sleep(2500); ws.close();
  ({ ws, ev } = await connect());
  const invoke2 = (cmd, args) => ev(`window.__TAURI__.core.invoke(${JSON.stringify(cmd)}, ${JSON.stringify(args || {})}).then(r=>JSON.stringify(r))`);

  console.log('— réflexion ACTIVÉE');
  await invoke2('core_reglage_set', { cle: 'reflexion', valeur: 'oui' });
  verifie(JSON.parse(await invoke2('core_reglages')).reflexion === true, 'le réglage est lu « oui »');
  const q = "J'ai 3 réunions de 45 minutes avec 10 minutes de pause entre chaque, la première commence à 9h20. À quelle heure finit la dernière ?";
  const a = await tour(ev, q, 240);
  console.log('  RÉFLEXION :', (a.think || '').slice(0, 300).replace(/\n/g, ' ⏎ '));
  console.log('  RÉPONSE   :', a.body);
  verifie(!a.err, 'pas d’erreur');
  verifie(!!a.think && a.think.length > 20, 'un bloc de réflexion a été rempli');
  verifie(a.open === false, 'le bloc est replié une fois la réponse commencée');
  verifie(a.body.length > 3 && !/<\/?(reflexion|think)/.test(a.body), 'la bulle contient la réponse, sans balise');
  verifie(!a.body.includes((a.think || 'x').slice(0, 40)), 'la réflexion n’est pas recopiée dans la bulle');

  console.log('— rechargement : la réflexion revient avec la conversation');
  await ev(`location.reload()`); await sleep(3000); ws.close();
  ({ ws, ev } = await connect());
  const r = JSON.parse(await ev(ETAT));
  verifie(!!r.think && r.open === false, 'bloc présent et replié après rechargement');
  verifie(r.body.length > 3, 'réponse présente après rechargement');

  console.log('— réflexion COUPÉE');
  const invoke3 = (cmd, args) => ev(`window.__TAURI__.core.invoke(${JSON.stringify(cmd)}, ${JSON.stringify(args || {})}).then(r=>JSON.stringify(r))`);
  await invoke3('core_reglage_set', { cle: 'reflexion', valeur: 'non' });
  const b = await tour(ev, 'Et si la première commence à 10h ?', 120);
  console.log('  RÉPONSE   :', b.body);
  verifie(b.think === null, 'aucun bloc de réflexion');
  verifie(b.body.length > 3 && !/<\/?(reflexion|think)/.test(b.body), 'réponse sans balise');
  ws.close();
  process.exit(ok ? 0 : 2);
})().catch((e) => { console.log('ERREUR:', e.message); process.exit(1); });
