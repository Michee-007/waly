# waly-seal — a kernel network seal for any local agent

**Block *all* outbound network traffic from a chosen program at the Windows
kernel level, keep loopback alive, and log every blocked attempt — verifiably.**

*« Huis clos » — the seal behind Waly's Guard: you see what AI touches, you
cut it off whenever you want.*

`waly-seal` is the network-seal brick from [Waly](../../README.md), usable on
its own. It was built to prove that a local AI assistant *cannot* phone home;
but the seal is program-agnostic — it can enclose **any** local agent or tool
(another AI agent, a scraper, an untrusted binary), not just Waly.

It is a small SYSTEM service driving the **Windows Filtering Platform (WFP)**
through `fwpuclnt.dll` (a Microsoft-signed system DLL — no third-party native
code, no C++ build scripts). For each sealed program it installs two kernel
filters: **permit loopback** (so a local model server keeps working) and
**block everything else**.

## Guarantees (and non-guarantees)

Honesty is part of the guarantee.

- **Kernel-level, per-executable.** Filtering is by exe *path*, at the ALE
  connect layer (v4/v6). Sealing an agent blocks every instance of that exe.
- **Fail-closed.** The WFP session is non-dynamic: if the service crashes, the
  filters *stay* (nothing leaks). They are reconciled on next start. The SCM is
  configured to restart the service after a crash, which re-seals idempotently.
- **Auditable.** Every blocked outbound attempt is a WFP `CLASSIFY_DROP` net
  event, drained into a local journal (exe, address:port, protocol, timestamp).
- **Sealing a third-party program requires elevation.** Blocking another
  program's network is a privileged act (a DoS vector otherwise), so the
  service refuses to seal anything outside Waly's own perimeter unless the
  caller is elevated. See
  [`docs/ADR-2026-09-16`](../../docs/ADR-2026-09-16-sceller-un-tiers-est-admin.md).
- It does **not** cover *shared* executables (e.g. a system WebView/browser
  runtime): filtering `msedgewebview2.exe` would break every app that uses it.
  Those are hardened differently (CSP, background-networking flags), not by WFP.
- It is **per-exe, not per-PID** (WFP does not filter by PID) — and **not
  per process tree**: a *different* program launched by a sealed one is not
  covered. If the agent you seal can spawn `curl`, a shell or an interpreter,
  seal those too, or use an identity-based sandbox (see *Related work*).
- **A program the service cannot see is refused, not silently skipped.**
  Joining the perimeter answers with an error when no filter could be
  installed. The service's "sealed" answer is still a declaration: prove it
  with a real attempt (Waly does, see `sceau::sonder` in `waly-core`).
- **UDP** send is dropped and journaled, but `sendto` returns success locally —
  probe the journal, not the return code.
- Windows only. A machine **administrator** can always remove the seal — but
  seal/unseal are journaled, so a coverage gap is visible.

## Build

From WSL, cross-compiling to native Windows (see the repo
[CONTRIBUTING.md](../../CONTRIBUTING.md) — Smart App Control blocks unsigned
`cargo build` build-scripts natively; the WSL→windows-gnu loop avoids it):

```bash
CARGO_TARGET_DIR=~/waly-target-wsl \
  cargo build --release -p waly-seal --bin waly-seal-svc \
  --target x86_64-pc-windows-gnu
```

The resulting `waly-seal-svc.exe` is both the service and its CLI. For
distribution it must be **code-signed** (a SYSTEM service and SAC verdicts).

## Install (elevated, once)

`setup` relocates the exe into `%ProgramFiles%\Waly` (a SYSTEM service must not
run from a user-writable path) and registers it as auto-start:

```powershell
# from an elevated console
waly-seal-svc.exe setup
```

Uninstall: `waly-seal-svc.exe uninstall` (elevated).

## Use — seal any local agent

```powershell
# From an ELEVATED console (sealing a third-party program needs elevation):
waly-seal-svc.exe seal  "C:\Path\To\some-agent.exe"   # block its egress
waly-seal-svc.exe unseal "C:\Path\To\some-agent.exe"  # restore it
waly-seal-svc.exe list                                 # sealed sessions
waly-seal-svc.exe journal "C:\Path\To\some-agent.exe" # its blocked attempts
```

Prove it: run the agent, watch it fail to reach the network, and read the
journal. A copy of `curl.exe` at a distinct path makes a clean stand-in — see
[`lab/huisclos-banc/banc-c.ps1`](../../lab/huisclos-banc/banc-c.ps1).

Measured on the reference machine: seal ~59 ms, unseal ~38 ms, zero resident
process, no loopback overhead.

## Related work — where it is done better

Confining an agent's network is an active field. Know the alternatives
before you choose this brick:

- **Anthropic `sandbox-runtime`** (Windows support in alpha) fences egress
  with WFP filters keyed on a **dedicated account's identity**, plus a local
  proxy that only lets declared domains through. That covers child processes
  and solves per-destination filtering — two things this brick does not do.
- **OpenAI Codex** ships a Windows sandbox built on dedicated sandbox users
  and firewall rules, for a coding agent.
- **NVIDIA OpenShell / NemoClaw** (alpha) confines personal agents on Linux,
  macOS and WSL 2, not on native Windows.
- **simplewall, Portmaster, Fort Firewall** are general per-application
  firewalls on the same Windows mechanism.

What this brick is for: a small, readable, fail-closed seal owned by a SYSTEM
service, with a narrow protocol and a journal — easy to audit, easy to embed
in an assistant. Details and sources:
[`docs/RESEARCH-2026-10-06-these-et-etat-de-l-art.md`](../../docs/RESEARCH-2026-10-06-these-et-etat-de-l-art.md).
Open problems and where to help:
[`docs/AUDIT-2026-10-02-promesses-rejouees.md`](../../docs/AUDIT-2026-10-02-promesses-rejouees.md).

## IPC (narrow by design)

The service listens on a named pipe (`\\.\pipe\waly-seal`), one JSON request
per line. The protocol only lets a client **seal/unseal/query its own
session** over a set of exe paths it names — **never** a raw filter definition,
so an app can never punch a hole in the seal. Types: [`src/ipc.rs`](src/ipc.rs).

## Layout

- [`src/wfp.rs`](src/wfp.rs) — the WFP engine (sublayer, permit/block pairs,
  net-event journal, fail-closed lifetime).
- [`src/service.rs`](src/service.rs) — SCM install/run, pipe server, the
  elevation check that gates third-party sealing.
- [`src/ipc.rs`](src/ipc.rs) — the narrow pipe protocol + pure helpers.
- [`src/bin/waly-seal-svc.rs`](src/bin/waly-seal-svc.rs) — service + CLI.

Design notes and measured gates:
[`docs/PLAN-2026-07-20-R6a-huis-clos.md`](../../docs/PLAN-2026-07-20-R6a-huis-clos.md),
[`docs/PLAN-2026-09-16-C-huis-clos-universel.md`](../../docs/PLAN-2026-09-16-C-huis-clos-universel.md),
[`docs/ONEPAGER-2026-07-21-huis-clos-conformite.md`](../../docs/ONEPAGER-2026-07-21-huis-clos-conformite.md).

## License

MIT, like the rest of Waly.
