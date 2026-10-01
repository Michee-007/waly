# Good first issues

Curated starting points for new contributors. Waly is young and
hardware-coupled: the most valuable early contributions are **measurements on
other machines**, **portability**, and **small, well-scoped features** with a
clear test. Each item below names the files to touch and how to verify.

See [CONTRIBUTING.md](../CONTRIBUTING.md) for the build loops and the
non-negotiable invariants (Rust-for-what-ships, 100 % native Windows, measured
exit criteria, privacy: no pixels / no raw OCR / no audio persisted).

## The network seal brick (`waly-seal`)

The seal (`crates/waly-seal`, see its [README](../crates/waly-seal/README.md))
is the most self-contained brick — a good place to start.

1. **`--json` output for the CLI.** `seal`/`unseal`/`list`/`journal` print
   human text; add a `--json` flag so the brick scripts cleanly against other
   tools. *Touch:* `crates/waly-seal/src/bin/waly-seal-svc.rs`. *Verify:* pipe
   into `jq`; the pure helpers already have unit tests in `src/ipc.rs`.

2. **`list` shows exe paths, not just session ids.** Today `Etat` returns
   session ids only; the app keeps paths in its own table. Extend the IPC
   `Etat` response to include the app-id path per sealed session so the CLI
   `list` is self-sufficient. *Touch:* `src/ipc.rs` (`Succes::Etat`),
   `src/wfp.rs` (already stores `app_ids`), `src/service.rs`.

3. **Config-file whitelist as an elevation alternative.** ADR
   [2026-09-16](ADR-2026-09-16-sceller-un-tiers-est-admin.md) chose
   admin-per-seal. Prototype the documented alternative: an admin-placed
   whitelist file of sealable programs that then allows non-elevated
   seal/unseal of *those* exes only. Keep it opt-in and fail-closed.

4. **`journal` follow mode (`--watch`).** Stream new blocked attempts as they
   happen (poll `Journal` on an interval, print deltas). *Touch:*
   `src/bin/waly-seal-svc.rs`.

5. **Interpreter-seal safety prompt.** `ressemble_interpreteur` already warns
   when sealing `python.exe`/`node.exe` (shared runtimes). Add a `--force`
   requirement before actually sealing one from the CLI. *Touch:*
   `src/ipc.rs` (helper exists), `src/bin/waly-seal-svc.rs`.

## Portability & measurements

6. **Measure a full turn on non-reference hardware.** Run the voice cascade /
   a chat turn on a different NPU/GPU/RAM profile and file the numbers. See
   `engines/README.md` for the measurement method and `docs/JOURNAL.md` for the
   format. This is the single most useful contribution right now.

7. **Generic OpenAI-compatible backend recipes.** `waly.toml` already targets
   any loopback OpenAI-compatible server (FastFlowLM, Ollama, llama.cpp, LM
   Studio) with automatic 42626 → 11434 fallback. Add per-engine setup recipes
   (llama.cpp, LM Studio) to `engines/README.md` and test one end-to-end.

## Docs & polish

8. **Screenshots / GIF for the README.** The repo has no demo media yet
   (see `ROADMAP.md` → "Public demo material"). A short capture of the seal's
   "verify now" flow, or the voice call, would help newcomers a lot.

9. **English mirror drift check.** `README.md` (EN) and `README.fr.md` (FR)
   can drift. A pass to reconcile them (content, not word-for-word) is easy and
   valuable.

---

*Filing your own: measurements, logs and portability reports are welcome even
without a linked issue — open one with your machine profile and what you saw.*
