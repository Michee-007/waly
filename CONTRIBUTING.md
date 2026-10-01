# Contributing to Waly

Thanks for your interest! Waly is young and hardware-coupled; the most
valuable contributions today are **measurements on other machines**, **bug
reports with logs**, and **portability work** — before new features.

New here? Pick an issue labelled
[`good first issue`](https://github.com/Michee-007/waly/issues?q=is%3Aissue+is%3Aopen+label%3A%22good+first+issue%22)
(the same list, with files to touch, is in
[docs/GOOD-FIRST-ISSUES.md](docs/GOOD-FIRST-ISSUES.md)). Comment on it before
you start, so nobody duplicates work. A first reply within two days is the
goal.

## Your first contribution, without the reference hardware

You do **not** need an NPU, a microphone, a camera or a model server to
contribute. Most of the logic is plain Rust with unit tests:

```bash
git clone https://github.com/Michee-007/waly.git && cd waly
# No system dependency needed for these crates (Linux, macOS or Windows):
cargo test -p waly-core -p waly-relais -p waly-seal
```

| Area | Needs | Where |
|---|---|---|
| Network seal CLI and service | Windows to run it, any OS to test helpers | `crates/waly-seal` |
| Sharing relay | nothing | `crates/waly-relais` |
| Chat loop, tools, router, privacy filters | nothing | `crates/waly-core` |
| English mode (strings, prompts, speech guards) | nothing to start | `apps/desktop/ui`, `crates/waly-core/src/prompt.rs` |
| Measurements | your own machine + Ollama | the *Measurement on my machine* issue form |

The desktop app end-to-end tests (`apps/desktop/e2e/`) run offline against
local servers; they need Windows, Ollama and a 4B model.

**How merges work.** Day-to-day development happens in the maintainer's
working tree and is published here as regular commits. Your pull request is
merged here like in any project; the maintainer then carries it into the
working tree before the next publication, so it is never overwritten.

The codebase, comments and `docs/` are largely in **French** (the project is
French-first). Contributions, issues and PRs in French or English are equally
welcome.

## Ground rules (from the architecture RFC)

- **Rust for everything that ships**; TypeScript/JS only inside the Tauri
  webview. No Node backend. SQLite (rusqlite + sqlite-vec) for storage.
- **100 % native Windows execution** — WSL is for editing/building only.
  Never run inference, audio or servers inside WSL.
- One resident LLM at a time; every new resident component must justify
  itself inside the ~6.5 GB AI RAM budget.
- Measured exit criteria per phase — see `docs/RFC-2026-07-03-*` and the
  `docs/PLAN-*` files. No claims without measurements.
- Privacy invariants are non-negotiable: no pixels, no raw OCR text, no
  audio ever persisted; network egress stays sealed by default.

## Build & test

Two loops, depending on your machine:

**Standard loop (most contributors):**
```bash
cargo check --workspace
cargo test --workspace
cargo build --release
```
CI runs exactly this on Linux (logic tests) and Windows.

**Smart App Control machines (like the reference machine):** SAC blocks
unsigned locally-built binaries unpredictably, including cargo build scripts.
The validated loop is to cross-compile from WSL and run on Windows:
```bash
CARGO_TARGET_DIR=~/waly-target-wsl cargo build --release --target x86_64-pc-windows-gnu
```
Details and hard-won pitfalls: `AGENTS.md` ("Pièges connus") and
`engines/README.md`. Never share a `target/` directory between WSL and
Windows.

**Tests:** `wsl.exe -e bash engines/run-tests-wsl.sh` on the reference
setup, or plain `cargo test --workspace` elsewhere. Logic tests must not
require a running model server, microphone, camera or elevated rights.

## Pull requests

- Keep PRs focused; one concern per PR.
- Add or update tests for behavior you change.
- If your change affects latency, RAM or the privacy invariants, include a
  before/after measurement (the repo culture is measurement-driven —
  see the ADR / PLAN files in `docs/` for the format).
- If your change opens a way out of the machine, it must go through the
  gateway (`exterieur::requete`), be something the user turns on, and be
  written to the seal journal. No exceptions.
- Document non-obvious decisions in an ADR under `docs/` if they are
  structural.

## Reporting security issues

Please do not open public issues for vulnerabilities — see
[SECURITY.md](SECURITY.md).
