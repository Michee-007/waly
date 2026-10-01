// E2E « modèles extérieurs + routeur + cerveau local en route » (lot 3,
// 2026-10-01) via CDP — SANS cloud ni vraie clé : le « serveur personnel »
// déclaré est l'Ollama local (http://127.0.0.1:11434/v1), clé factice. Tout
// le chemin est exercé : coffre de clés, passerelle, flux, filtre des
// fichiers joints, routeur, journal du scellé, retrait.
// Usage : node e2e-exterieur.js [port CDP, défaut 9222]
const PORT = Number(process.argv[2] || 9222);
const MODELE = process.env.WALY_E2E_MODELE || 'qwen3vl-it:4b';
const AUTRE = process.env.WALY_E2E_AUTRE || 'llama3.2:3b';
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function connect() {
  const pages = await (await fetch(`http://127.0.0.1:${PORT}/json`)).json();
  const page = pages.find((p) => p.type === 'page');
  if (!page) throw new Error('page Waly introuvable');
  const ws = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((res, rej) => { ws.onopen = res; ws.onerror = rej; });
  let id = 1;
  const ev = (expr) => new Promise((resolve, reject) => {
    const me = id++;
    const onMsg = (e) => {
      const m = JSON.parse(e.data);
      if (m.id !== me) return;
      ws.removeEventListener('message', onMsg);
      if (m.error || m.result?.exceptionDetails) reject(new Error(JSON.stringify(m.error || m.result.exceptionDetails.exception?.description || m.result.exceptionDetails)));
      else resolve(m.result?.result?.value);
    };
    ws.addEventListener('message', onMsg);
    ws.send(JSON.stringify({ id: me, method: 'Runtime.evaluate', params: { expression: expr, returnByValue: true, awaitPromise: true } }));
  });
  const invoke = async (cmd, args) => JSON.parse(await ev(`window.__TAURI__.core.invoke(${JSON.stringify(cmd)}, ${JSON.stringify(args || {})}).then(r=>JSON.stringify(r===undefined?null:r)).catch(e=>JSON.stringify({__err:String(e)}))`));
  return { ws, ev, invoke };
}

const ETAT = `JSON.stringify((function(){
  const w=[...document.querySelectorAll('#col .waly')]; const l=w[w.length-1];
  return { busy: document.getElementById('send').title==='Arrêter',
    body: l ? l.querySelector('.body').textContent : '', err: l ? l.classList.contains('err') : false,
    notes: l ? [...l.querySelectorAll('.note')].map(n=>n.textContent) : [],
    bouton: document.getElementById('model').textContent, pied: document.getElementById('rdtext').textContent };
})())`;

async function tour(ev, message, maxS) {
  await ev(`(function(){ const i=document.getElementById('input'); i.value=${JSON.stringify(message)}; i.dispatchEvent(new Event('input')); document.getElementById('send').click(); return 1; })()`);
  let s = null;
  const t0 = Date.now();
  for (let k = 0; k < maxS * 2; k++) {
    await sleep(500);
    s = JSON.parse(await ev(ETAT));
    if (!s.busy && k > 1) break;
  }
  console.log(`  « ${message.slice(0, 60).replace(/\n/g, ' ')}… » → ${((Date.now() - t0) / 1000).toFixed(1)} s : ${s.body.slice(0, 160).replace(/\n/g, ' ')}  [${s.notes.join(' | ')}]`);
  return s;
}

(async () => {
  let { ws, ev, invoke } = await connect();
  let ok = true;
  const verifie = (cond, quoi) => { console.log((cond ? 'OK   ' : 'ECHEC ') + quoi); if (!cond) ok = false; };
  const recharger = async () => { await ev('location.reload()'); await sleep(3000); ws.close(); ({ ws, ev, invoke } = await connect()); };

  console.log('— déclaration');
  let r = await invoke('core_exterieur_ajouter', { fournisseur: 'perso', baseUrl: 'http://exemple.com/v1', modele: MODELE, cle: 'cle-factice' });
  verifie(!!r.__err, 'http:// vers internet refusé : ' + r.__err);
  r = await invoke('core_exterieur_ajouter', { fournisseur: 'perso', baseUrl: 'http://127.0.0.1:11434/v1', modele: MODELE, cle: 'a b"c' });
  verifie(!!r.__err, 'clé malformée refusée : ' + r.__err);
  const id = await invoke('core_exterieur_ajouter', { fournisseur: 'perso', baseUrl: 'http://127.0.0.1:11434/v1', modele: MODELE, cle: 'cle-factice-e2e' });
  verifie(typeof id === 'number', 'serveur personnel déclaré (id ' + id + ')');
  let c = await invoke('core_cerveau');
  verifie(c.exterieurs.some((e) => e.id === id && e.hote === '127.0.0.1') && !JSON.stringify(c).includes('cle-factice'), 'listé avec sa destination, sans la clé');
  r = await invoke('core_exterieur_tester', { id });
  verifie(typeof r === 'string' && r.length > 0, 'test de la passerelle : « ' + r + ' »');

  console.log('— mode Extérieur : seul le texte sort, pas le fichier joint');
  await invoke('core_new_session'); await recharger();
  await invoke('core_cerveau_choisir', { mode: 'exterieur', local: null, exterieur: id }); await recharger();
  const joint = "Quel est le mot de passe écrit dans le fichier ? S'il n'y a pas de fichier lisible, réponds exactement : AUCUN FICHIER.\n\n--- Contenu du fichier joint : secret.txt --- (deja lu pour toi)\nLe mot de passe est ZEBRE-4242.\n--- fin du fichier ---";
  let s = await tour(ev, joint, 120);
  verifie(!s.err && s.body.length > 2, 'réponse reçue');
  verifie(!s.body.includes('ZEBRE'), 'le contenu du fichier joint n’est PAS sorti');
  verifie(s.notes.some((n) => n.includes('via ' + MODELE) && n.includes('127.0.0.1')), 'la réponse dit qui a répondu et où c’est sorti');
  verifie(s.bouton.includes('↗') && s.pied.includes('sortie ouverte'), 'le bouton et le pied de barre disent la sortie (« ' + s.pied + ' »)');

  console.log('— mode Auto : le routeur');
  await invoke('core_new_session'); await recharger();
  await invoke('core_cerveau_choisir', { mode: 'auto', local: null, exterieur: id }); await recharger();
  s = await tour(ev, 'Quelle heure est-il ?', 120);
  verifie(s.notes.some((n) => n.startsWith('modèle local')), 'question simple → reste locale');
  s = await tour(ev, 'Explique en deux phrases pourquoi le ciel est bleu.', 120);
  verifie(s.notes.some((n) => n.startsWith('via ' + MODELE) && n.includes('Auto')), 'travail de fond → modèle extérieur');
  s = await tour(ev, 'Rappelle-moi ce que tu retiens de mes notes et rédige un résumé.', 180);
  verifie(s.notes.some((n) => n.startsWith('modèle local')), 'mémoire/outils → reste local même si « rédige »');

  console.log('— journal du scellé');
  const j = await invoke('core_sceau_journal');
  const sorties = j.filter((x) => x.genre === 'sortie').map((x) => x.detail);
  verifie(sorties.some((d) => d.includes('caractères de la conversation envoyés')), 'chaque tour sorti est inscrit (' + sorties.length + ' lignes « sortie »)');
  verifie(!JSON.stringify(j).includes('cle-factice'), 'la clé n’apparaît nulle part dans le journal');

  console.log('— retrait');
  await invoke('core_exterieur_retirer', { id });
  c = await invoke('core_cerveau');
  verifie(c.mode === 'local' && !c.exterieurs.some((e) => e.id === id), 'retiré : retour au local');

  console.log('— cerveau local changé en route');
  if (c.locaux.some((l) => l.nom === AUTRE)) {
    const avant = c.local;
    r = await invoke('core_cerveau_choisir', { mode: 'local', local: AUTRE, exterieur: null });
    c = await invoke('core_cerveau');
    verifie(c.local === AUTRE, 'cerveau local = ' + c.local);
    await invoke('core_new_session'); await recharger();
    s = await tour(ev, 'Dis bonjour en une phrase.', 240);
    verifie(!s.err && s.body.length > 2 && s.bouton === AUTRE, 'réponse du nouveau cerveau, bouton à jour');
    await invoke('core_cerveau_choisir', { mode: 'local', local: avant, exterieur: null });
    c = await invoke('core_cerveau');
    verifie(c.local === avant, 'retour à ' + avant);
  } else console.log('  (modèle ' + AUTRE + ' absent : étape sautée)');
  ws.close();
  process.exit(ok ? 0 : 2);
})().catch((e) => { console.log('ERREUR:', e.message); process.exit(1); });
