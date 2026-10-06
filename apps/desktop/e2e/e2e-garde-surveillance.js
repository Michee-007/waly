// E2E « la Garde, étape 3 : la surveillance », sur la vraie app et le service
// INSTALLÉ (CDP). Allume « Surveiller toute la machine » par le bouton
// (Windows demande l'accord), laisse un programme connu faire quatre gestes,
// vérifie qu'ils arrivent dans le fil de la Garde, puis éteint.
// Usage : node e2e-garde-surveillance.js [fichier de capture] [port CDP]
// Le programme connu : PowerShell lit C:\waly\README.md, crée un fichier,
// lance ping et se connecte à 1.1.1.1:443.
import { writeFileSync } from 'node:fs';
import { spawn } from 'node:child_process';

const CAPTURE = process.argv[2] || '';
const PORT = Number(process.argv[3] || 9222);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const pages = await (await fetch(`http://127.0.0.1:${PORT}/json`)).json();
const ws = new WebSocket(pages.find((p) => p.type === 'page').webSocketDebuggerUrl);
await new Promise((res, rej) => { ws.onopen = res; ws.onerror = rej; });
let id = 1;
const cdp = (method, params) => new Promise((resolve, reject) => {
  const me = id++;
  const onMsg = (e) => { const m = JSON.parse(e.data); if (m.id !== me) return; ws.removeEventListener('message', onMsg); m.error ? reject(new Error(JSON.stringify(m.error))) : resolve(m.result); };
  ws.addEventListener('message', onMsg);
  ws.send(JSON.stringify({ id: me, method, params: params || {} }));
});
const ev = async (expr) => (await cdp('Runtime.evaluate', { expression: expr, returnByValue: true, awaitPromise: true })).result?.value;
const invoke = async (cmd, args) => JSON.parse(await ev(`window.__TAURI__.core.invoke(${JSON.stringify(cmd)}, ${JSON.stringify(args || {})}).then(r=>JSON.stringify(r===undefined?null:r)).catch(e=>JSON.stringify({__err:String(e)}))`));
const attendre = async (cond, maxS) => { for (let i = 0; i < maxS * 2; i++) { const v = await cond(); if (v) return v; await sleep(500); } return null; };
let echecs = 0;
const dire = (ok, quoi, detail) => { if (!ok) echecs++; console.log(`${ok ? 'OK  ' : 'ECHEC'} ${quoi}${detail ? ' — ' + detail : ''}`); };

await cdp('Emulation.setDeviceMetricsOverride', { width: 1500, height: 1000, deviceScaleFactor: 1, mobile: false });
if (!(await ev(`(function(){ const c=document.getElementById('gdcarte'); return !!c && c.offsetParent!==null; })()`))) { await ev(`document.getElementById('gardebtn').click()`); await sleep(2000); }

let g = await invoke('core_garde');
dire(g.regard.mode === 'rien', 'la surveillance est éteinte au départ', g.regard.erreur || g.regard.mode);
await ev(`document.getElementById('gdrtout').click()`); await sleep(400);
console.log('     dialogue : ' + await ev(`document.getElementById('dlgtitle').textContent+' | '+document.getElementById('dlgtext').textContent`));
await ev(`document.getElementById('dlgok').click()`);
const allumee = await attendre(async () => (await invoke('core_garde')).regard.mode === 'tout', 150);
dire(!!allumee, 'surveillance de toute la machine allumée depuis la Garde', await ev(`document.getElementById('gdvoix').textContent`));

if (allumee) {
  await sleep(1500);
  const gestes = "Start-Sleep -Milliseconds 800; $null = Get-Content 'C:\\waly\\README.md'; Set-Content 'C:\\waly\\lab\\garde-banc\\essai-surveillance.txt' 'essai'; Start-Process ping.exe -ArgumentList '-n','1','127.0.0.1' -WindowStyle Hidden -Wait; try { $c = New-Object Net.Sockets.TcpClient; $c.Connect('1.1.1.1',443); $c.Close() } catch {}; Start-Sleep -Milliseconds 800; Remove-Item 'C:\\waly\\lab\\garde-banc\\essai-surveillance.txt'";
  await new Promise((res) => spawn('powershell.exe', ['-NoProfile', '-Command', gestes], { stdio: 'ignore', windowsHide: true }).on('exit', res));
  // La page relève le service toutes les 2,5 s et inscrit ce qu'il a vu.
  const vus = (fil) => ({
    lecture: fil.some((x) => /a ouvert .*waly\\README\.md/i.test(x.detail)),
    ecriture: fil.some((x) => /a (créé|écrit) .*essai-surveillance\.txt/i.test(x.detail)),
    lancement: fil.some((x) => /a lancé .*PING\.EXE/i.test(x.detail)),
    connexion: fil.some((x) => /connecté à 1\.1\.1\.1:443/.test(x.detail)),
  });
  let v = null;
  await attendre(async () => { v = vus((await invoke('core_garde')).fil); return v.lecture && v.ecriture && v.lancement && v.connexion; }, 20);
  g = await invoke('core_garde');
  dire(v.lecture, 'la lecture de README.md est dans le fil');
  dire(v.ecriture, 'la création du fichier est dans le fil');
  dire(v.lancement, 'le lancement de ping est dans le fil');
  dire(v.connexion, 'la connexion à 1.1.1.1:443 est dans le fil');
  const autres = [...new Set(g.fil.filter((x) => x.agent !== 'Waly' && x.agent !== 'Toi').map((x) => x.agent))];
  console.log(`     ${g.fil.length} lignes au fil ; programmes vus : ${autres.slice(0, 12).join(', ')}`);
  for (const x of g.fil.filter((l) => /powershell/i.test(l.agent)).slice(0, 8)) console.log(`     ${x.at.slice(11)} ${x.agent} ${x.detail}`);
  if (CAPTURE) {
    await ev(`(function(){ const b=document.getElementById('gdfiltout'); if(b && !b.hidden && /toute la machine/.test(b.textContent)) b.click(); const l=document.getElementById('gdfiltitre'); if(l) l.scrollIntoView({block:'start'}); return 1; })()`);
    await sleep(1200);
    writeFileSync(CAPTURE, Buffer.from((await cdp('Page.captureScreenshot', { format: 'png' })).data, 'base64'));
    console.log('     capture : ' + CAPTURE);
  }
}
// Éteindre : permis à tous, sans accord de Windows.
await ev(`(function(){ const b=document.getElementById('gdroff'); if(b) b.click(); return 1; })()`);
const eteinte = await attendre(async () => (await invoke('core_garde')).regard.mode === 'rien', 20);
if (!eteinte) await invoke('core_regard', { mode: 'rien', exes: [] });
dire(!!eteinte, 'surveillance éteinte par le bouton, sans accord de Windows');
await cdp('Emulation.clearDeviceMetricsOverride');
ws.close();
console.log(echecs ? `\n${echecs} échec(s)` : '\nTout est passé.');
process.exit(echecs ? 1 : 0);
