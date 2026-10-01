// E2E « partage entre deux Waly » (lot 3, 2026-10-01) via CDP : DEUX
// instances de l'app (Alice : CDP 9222, Bob : CDP 9223, deux bases) et un
// relais local `waly-relais`. Vérifie : identités et codes, enveloppe d'un
// inconnu jetée, conversation reçue EN ATTENTE puis acceptée à l'identique,
// refus, enveloppe falsifiée jetée, journal du scellé des deux côtés.
// Usage : node e2e-partage.js [relais, défaut http://127.0.0.1:18787]
const RELAIS = process.argv[2] || 'http://127.0.0.1:18787';
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function instance(port) {
  const pages = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
  const ws = new WebSocket(pages.find((p) => p.type === 'page').webSocketDebuggerUrl);
  await new Promise((res, rej) => { ws.onopen = res; ws.onerror = rej; });
  let id = 1;
  const ev = (expr) => new Promise((resolve) => {
    const me = id++;
    const onMsg = (e) => { const m = JSON.parse(e.data); if (m.id !== me) return; ws.removeEventListener('message', onMsg); resolve(m.result?.result?.value); };
    ws.addEventListener('message', onMsg);
    ws.send(JSON.stringify({ id: me, method: 'Runtime.evaluate', params: { expression: expr, returnByValue: true, awaitPromise: true } }));
  });
  const invoke = async (cmd, args) => JSON.parse(await ev(`window.__TAURI__.core.invoke(${JSON.stringify(cmd)}, ${JSON.stringify(args || {})}).then(r=>JSON.stringify(r===undefined?null:r)).catch(e=>JSON.stringify({__err:String(e)}))`));
  return { ws, ev, invoke };
}
const attendre = async (cond, maxS) => { for (let i = 0; i < maxS * 2; i++) { if (await cond()) return true; await sleep(500); } return false; };

(async () => {
  const A = await instance(9222), B = await instance(9223);
  let ok = true;
  const verifie = (cond, quoi) => { console.log((cond ? 'OK   ' : 'ECHEC ') + quoi); if (!cond) ok = false; };
  const sorties = async (X) => (await X.invoke('core_sceau_journal')).filter((x) => x.genre === 'sortie').map((x) => x.detail);
  for (const X of [A, B]) for (const c of (await X.invoke('core_partage')).contacts) await X.invoke('core_contact_retirer', { id: c.id });

  console.log('— identités');
  let r = await A.invoke('core_partage_regler', { relais: 'http://relais.exemple.com', nom: 'Alice' });
  verifie(!!r.__err, 'relais http:// vers internet refusé : ' + r.__err);
  await A.invoke('core_partage_regler', { relais: RELAIS, nom: 'Alice' });
  await B.invoke('core_partage_regler', { relais: RELAIS, nom: 'Bob' });
  const a = await A.invoke('core_partage'), b = await B.invoke('core_partage');
  verifie(/^waly1-[a-z2-7]+@http/.test(a.code) && a.code !== b.code, 'chaque installation a son code Waly (clé publique + boîte + relais)');
  r = await A.invoke('core_contact_ajouter', { nom: 'Moi', code: a.code });
  verifie(!!r.__err, 'on ne s’ajoute pas soi-même : ' + r.__err);
  r = await A.invoke('core_contact_ajouter', { nom: 'Bob', code: b.code.replace(/.(?=.{40}@)/, (c) => (c === 'a' ? 'b' : 'a')) });
  verifie(!!r.__err, 'code mal recopié refusé : ' + r.__err);
  const idBob = await A.invoke('core_contact_ajouter', { nom: 'Bob', code: b.code });
  verifie(typeof idBob === 'number', 'Alice ajoute Bob');

  console.log('— une conversation chez Alice');
  await A.invoke('core_new_session'); await A.ev('location.reload()'); await sleep(3000); A.ws.close();
  const A2 = await instance(9222);
  await A2.ev(`(function(){ const i=document.getElementById('input'); i.value='Réponds en un mot : la capitale du Sénégal ?'; i.dispatchEvent(new Event('input')); document.getElementById('send').click(); return 1; })()`);
  await attendre(async () => (await A2.invoke('core_history'))[1].length >= 2, 120);
  const original = (await A2.invoke('core_history'))[1];
  verifie(original.length >= 2, 'conversation prête : « ' + original[1][1].slice(0, 40) + ' »');

  console.log('— Bob ne connaît pas encore Alice');
  // La boîte de Bob s'ouvre à sa première relève (fil de fond).
  let envoi = null;
  await attendre(async () => { envoi = await A2.invoke('core_partage_envoyer', { contact: idBob, titre: 'Capitale' }); return !envoi.__err; }, 40);
  verifie(envoi === 'Bob', 'Alice envoie (dépôt accepté par le relais)');
  await attendre(async () => (await sorties(B)).some((d) => d.includes('jetée')), 40);
  verifie((await sorties(B)).some((d) => d.includes('jetée')) && (await B.invoke('core_partage')).recus.length === 0, 'chez Bob : enveloppe d’un inconnu jetée, rien en attente');

  console.log('— Bob ajoute Alice : reçu, en attente, accepté');
  await B.invoke('core_contact_ajouter', { nom: 'Alice', code: a.code });
  await A2.invoke('core_partage_envoyer', { contact: idBob, titre: 'Capitale' });
  verifie(await attendre(async () => (await B.invoke('core_partage')).recus.length === 1, 40), 'chez Bob : une conversation en attente de son accord');
  let recu = (await B.invoke('core_partage')).recus[0];
  verifie(recu.contact === 'Alice' && recu.titre === 'Capitale' && recu.messages === original.length, `« ${recu.titre} » de ${recu.contact}, ${recu.messages} messages`);
  const avant = (await B.invoke('core_sessions', { kind: 'all' })).length;
  const session = await B.invoke('core_partage_decider', { id: recu.id, accepter: true });
  const apres = await B.invoke('core_sessions', { kind: 'all' });
  verifie(apres.length === avant + 1 && apres.some((s) => s[0] === session && s[1] === 'Capitale · reçue de Alice'), 'acceptée : nouvelle conversation « Capitale · reçue de Alice »');
  const copie = await B.invoke('core_select_session', { id: session });
  verifie(JSON.stringify(copie) === JSON.stringify(original), 'contenu identique à l’original, message pour message');

  console.log('— refus, falsification');
  await A2.invoke('core_partage_envoyer', { contact: idBob, titre: 'À refuser' });
  await attendre(async () => (await B.invoke('core_partage')).recus.length === 1, 40);
  recu = (await B.invoke('core_partage')).recus[0];
  await B.invoke('core_partage_decider', { id: recu.id, accepter: false });
  verifie((await B.invoke('core_partage')).recus.length === 0 && (await B.invoke('core_sessions', { kind: 'all' })).length === apres.length, 'refusée : effacée, aucune conversation créée');
  // Quelqu'un qui connaît la boîte de Bob y dépose une fausse enveloppe « d'Alice ».
  const boiteBob = await B.ev(`window.__TAURI__.core.invoke('core_partage').then(p=>p.code)`);
  const jetesAvant = (await sorties(B)).filter((d) => d.includes('jetée')).length;
  const contactAlice = (await B.invoke('core_partage')).contacts[0];
  verifie(!!contactAlice && contactAlice.empreinte.length === 8, 'empreinte de la clé d’Alice affichée chez Bob : ' + (contactAlice || {}).empreinte);
  // (Le test ne connaît pas l'identifiant de boîte en clair : il passe par le code de Bob.)
  const b32 = 'abcdefghijklmnopqrstuvwxyz234567';
  const corps = boiteBob.slice(6, boiteBob.indexOf('@'));
  let bits = '', octets = [];
  for (const c of corps) bits += b32.indexOf(c).toString(2).padStart(5, '0');
  for (let i = 0; i + 8 <= bits.length; i += 8) octets.push(parseInt(bits.slice(i, i + 8), 2));
  const hex = (o) => o.map((x) => x.toString(16).padStart(2, '0')).join('');
  const boite = hex(octets.slice(32, 48));
  const cleAlice = (() => { const ca = a.code.slice(6, a.code.indexOf('@')); let bb = '', oo = []; for (const c of ca) bb += b32.indexOf(c).toString(2).padStart(5, '0'); for (let i = 0; i + 8 <= bb.length; i += 8) oo.push(parseInt(bb.slice(i, i + 8), 2)); return hex(oo.slice(0, 32)); })();
  const faux = JSON.stringify({ v: 1, de: cleAlice, n: '00'.repeat(24), c: 'ab'.repeat(64) });
  const depot = await fetch(`${RELAIS}/v1/boites/${boite}`, { method: 'POST', body: faux });
  verifie(depot.status === 201, 'fausse enveloppe « d’Alice » déposée directement au relais');
  await attendre(async () => (await sorties(B)).filter((d) => d.includes('jetée')).length > jetesAvant, 40);
  verifie((await sorties(B)).filter((d) => d.includes('jetée')).length > jetesAvant && (await B.invoke('core_partage')).recus.length === 0, 'chez Bob : la fausse enveloppe est jetée (signature d’Alice impossible à imiter)');
  const lecture = await fetch(`${RELAIS}/v1/boites/${boite}`, { headers: { Authorization: 'Bearer ' + 'c'.repeat(64) } });
  verifie(lecture.status === 403, 'le relais refuse de livrer la boîte de Bob à un autre jeton');

  console.log('— journal du scellé');
  const ja = await sorties(A2), jb = await sorties(B);
  verifie(ja.some((d) => d.includes('envoyée à Bob')) && jb.some((d) => d.includes('reçue de Alice')), 'envoi inscrit chez Alice, réception inscrite chez Bob');

  for (const X of [A2, B]) { for (const c of (await X.invoke('core_partage')).contacts) await X.invoke('core_contact_retirer', { id: c.id }); await X.invoke('core_partage_regler', { relais: '', nom: '' }); }
  verifie((await A2.invoke('core_partage')).code === null, 'partage coupé : plus de relève');
  A2.ws.close(); B.ws.close();
  process.exit(ok ? 0 : 2);
})().catch((e) => { console.log('ERREUR:', e.message); process.exit(1); });
