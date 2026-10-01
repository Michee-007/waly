# Contributing to Waly

Thanks for your interest! Waly is young and hardware-coupled; the most
valuable contributions today are **measurements on other machines**, **bug
reports with logs**, and **portability work** — before new features.

New here? See [docs/GOOD-FIRST-ISSUES.md](docs/GOOD-FIRST-ISSUES.md) for
scoped starting points (the `waly-seal` network-seal brick is the most
self-contained one).

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
- Document non-obvious decisions in an ADR under `docs/` if they are
  structural.

## Reporting security issues

Please do not open public issues for vulnerabilities — see
[SECURITY.md](SECURITY.md).
