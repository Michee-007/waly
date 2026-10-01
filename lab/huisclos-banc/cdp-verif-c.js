// Verifie via CDP l'UI « Sceller un autre agent » (chantier C2) :
// - core_agents repond (registre vide au depart)
// - la pane Vie privee rend la section sans erreur JS
// Prerequis : Waly.exe lance avec --remote-debugging-port=9222.
const CDP = 'http://127.0.0.1:9222';

async function main() {
  const list = await (await fetch(CDP + '/json')).json();
  const page = list.find(p => p.type === 'page' && p.webSocketDebuggerUrl);
  if (!page) throw new Error('aucune page CDP');
  const ws = new WebSocket(page.webSocketDebuggerUrl);
  let id = 0;
  const pending = new Map();
  const send = (method, params) => new Promise((res, rej) => {
    const m = ++id; pending.set(m, { res, rej });
    ws.send(JSON.stringify({ id: m, method, params }));
  });
  await new Promise(r => ws.addEventListener('open', r));
  ws.addEventListener('message', ev => {
    const msg = JSON.parse(ev.data);
    if (msg.id && pending.has(msg.id)) { pending.get(msg.id).res(msg); pending.delete(msg.id); }
  });
  await send('Runtime.enable');

  const evalJs = async (expr) => {
    const r = await send('Runtime.evaluate', {
      expression: `(async()=>{ ${expr} })()`,
      awaitPromise: true, returnByValue: true,
    });
    if (r.result && r.result.exceptionDetails) throw new Error(JSON.stringify(r.result.exceptionDetails));
    if (r.result && r.result.result && r.result.result.subtype === 'error')
      throw new Error(r.result.result.description);
    return r.result.result.value;
  };

  // 1) commande enregistree + migration OK
  const agents = await evalJs(`const a = await window.__TAURI__.core.invoke('core_agents'); return JSON.stringify(a);`);
  console.log('core_agents ->', agents);

  // 2) etat du sceau (perimetre)
  const etat = await evalJs(`return JSON.stringify(await window.__TAURI__.core.invoke('core_sceau_etat'));`);
  console.log('core_sceau_etat ->', etat);

  // 3) naviguer par de VRAIS clics : ouvrir les reglages puis l'onglet Vie
  // privee, et verifier que la section rend (innerHTML sous CSP a empreintes).
  await evalJs(`document.querySelector('#me')?.click(); return true;`);
  await new Promise(r => setTimeout(r, 400));
  await evalJs(`document.querySelector('[data-sp="vieprivee"]')?.click(); return true;`);
  await new Promise(r => setTimeout(r, 700)); // laisse core_agents/core_sceau_etat repondre
  const rendu = await evalJs(`
    const sp = document.querySelector('#sp') || document.body;
    const html = sp.innerHTML;
    return JSON.stringify({
      titre_section: html.includes('Sceller un autre agent'),
      bouton_choisir: !!document.querySelector('#scadd'),
      liste: !!document.querySelector('#scagents'),
      liste_txt: (document.querySelector('#scagents')?.textContent || '').slice(0,80),
    });
  `);
  console.log('rendu pane Vie privee ->', rendu);
  ws.close();
}
main().then(() => process.exit(0)).catch(e => { console.error('ECHEC:', e.message); process.exit(1); });
