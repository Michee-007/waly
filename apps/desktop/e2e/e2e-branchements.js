// E2E des branchements R3 : sessions (Nouveau, titre auto, bascule),
// recherche locale, panneaux Competences/Artefacts, espace Agentique honnete.
// Prerequis : app lancee avec --remote-debugging-port=9222 et WALY_DB jetable.
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
  const convs = () => E(`JSON.stringify([...document.querySelectorAll('.conv .t')].map(e=>e.textContent))`);
  const waitIdle = async () => { for (let i=0;i<120;i++){ await sleep(500); if (await E(`document.getElementById('send').textContent`) !== '◼') return; } };

  // 1. Demarrage : base vierge -> une seule session « Fil principal »
  await sleep(500);
  check('fil principal au boot', JSON.parse(await convs()).join(',') === 'Fil principal');

  // 2. Nouveau -> deuxieme session, active
  await E(`document.getElementById('new').click()`);
  await sleep(400);
  check('nouveau cree une session', JSON.parse(await convs()).length === 2);

  // 3. Premier message -> titre auto de la session + reponse
  await E(`document.getElementById('input').value='Parle-moi du fromage comte en une phrase.';document.getElementById('send').click()`);
  await waitIdle();
  const titres = JSON.parse(await convs());
  check('titre auto depuis le 1er message', titres.some(t => t.startsWith('Parle-moi du fromage')), titres.join(' | '));
  const nbMsg = await E(`document.querySelectorAll('.msg').length`);
  check('tour complet rendu', nbMsg === 2);

  // 4. Bascule vers le Fil principal -> vide ; retour -> historique la
  await E(`[...document.querySelectorAll('.conv')].find(c=>c.textContent.includes('Fil principal')).click()`);
  await sleep(500);
  check('fil principal vide (cloisonnement)', await E(`!!document.querySelector('.empty')`) === true);
  await E(`[...document.querySelectorAll('.conv')].find(c=>c.textContent.includes('fromage')).click()`);
  await sleep(500);
  check('retour session : historique recharge', await E(`document.querySelectorAll('.msg').length`) === 2);

  // 5. Recherche locale : « comte » -> 1 session ; vider -> 2
  await E(`document.getElementById('searchbtn').click()`);
  await E(`const i=document.getElementById('sinput'); i.value='comte'; i.dispatchEvent(new Event('input'))`);
  await sleep(600);
  check('recherche filtre', JSON.parse(await convs()).length === 1);
  await E(`document.getElementById('searchbtn').click()`); // ferme + reset
  await sleep(500);
  check('recherche fermee restaure', JSON.parse(await convs()).length === 2);

  // 6. Competences : outils reels du registre
  await E(`document.getElementById('skills').click()`);
  await sleep(500);
  const nSkills = await E(`document.querySelectorAll('#cbody .crow').length`);
  check('competences listent les outils reels', nSkills >= 10, nSkills + ' outils');
  await E(`document.getElementById('cclose').click()`);

  // 7. Artefacts : trois sections honnetes (base vierge -> vides)
  await E(`document.getElementById('artifacts').click()`);
  await sleep(500);
  const grps = await E(`JSON.stringify([...document.querySelectorAll('#cbody .cgrp')].map(e=>e.textContent))`);
  const vides = await E(`document.querySelectorAll('#cbody .cempty').length`);
  // Carnet (ex-Artefacts) : Documents + Notes + Taches + Rappels (Mains v1).
  check('carnet : 4 sections dont Documents', JSON.parse(grps).length === 4, grps);
  check('carnet honnete sur base vierge', vides === 4);
  await E(`document.getElementById('cclose').click()`);

  // 8. Espace UNIQUE (fusion) : brouillon de mission puis retour a la
  //    conversation par la liste — l'historique est restaure.
  await E(`document.getElementById('newmission').click()`);
  await sleep(400);
  check('brouillon de mission : titre', await E(`document.getElementById('mtitle').textContent`) === 'Nouvelle mission');
  check('brouillon : etat vide affiche', await E(`!!document.querySelector('.empty')`) === true);
  await E(`document.querySelector('.conv').click()`);
  await sleep(600);
  check('retour conversation : historique restaure', await E(`document.querySelectorAll('.msg').length`) === 2);

  console.log(failures === 0 ? 'TOUT VERT' : failures + ' echec(s)');
  process.exit(failures === 0 ? 0 : 1);
})().catch(e => { console.log('ERREUR:', e.message); process.exit(1); });
