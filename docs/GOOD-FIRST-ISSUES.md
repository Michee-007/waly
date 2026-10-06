# Good first issues

Curated starting points for new contributors. They are also open as GitHub
issues labelled
[`good first issue`](https://github.com/Michee-007/waly/issues?q=is%3Aissue+is%3Aopen+label%3A%22good+first+issue%22)
— comment on one before you start so nobody duplicates work.

Waly is young and hardware-coupled: the most valuable early contributions
are **measurements on other machines**, **portability**, and **small,
well-scoped features** with a clear test. Each item names the files to touch
and how to verify.

See [CONTRIBUTING.md](../CONTRIBUTING.md) for the build loops and the
non-negotiable invariants.

## No special hardware needed

These build and test on any Linux or Windows machine, with no model server,
microphone or camera.

### The network seal brick (`waly-seal`)

The seal (`crates/waly-seal`, see its [README](../crates/waly-seal/README.md))
is the most self-contained brick, and it is useful to other local agents.

1. **`--json` output for the CLI.** `seal`/`unseal`/`list`/`journal` print
   human text; add a `--json` flag so the brick scripts cleanly against other
   tools. *Touch:* `crates/waly-seal/src/bin/waly-seal-svc.rs`. *Verify:* pipe
   into `jq`; the pure helpers already have unit tests in `src/ipc.rs`.

2. **`list` shows exe paths, not just session ids.** Extend the IPC `Etat`
   response to include the app-id path per sealed session so the CLI `list`
   is self-sufficient. *Touch:* `src/ipc.rs` (`Succes::Etat`), `src/wfp.rs`
   (already stores `app_ids`), `src/service.rs`.

3. **`journal` follow mode (`--watch`).** Stream new blocked attempts as they
   happen (poll `Journal` on an interval, print deltas). *Touch:*
   `src/bin/waly-seal-svc.rs`.

4. **Interpreter-seal safety prompt.** `ressemble_interpreteur` already warns
   when sealing `python.exe`/`node.exe` (shared runtimes). Require `--force`
   before actually sealing one from the CLI. *Touch:* `src/ipc.rs`,
   `src/bin/waly-seal-svc.rs`.

### Open design problems (bigger, and where we most need other minds)

These are not small tasks. They are the open problems listed in
[`AUDIT-2026-10-02-promesses-rejouees.md`](AUDIT-2026-10-02-promesses-rejouees.md),
each with the direction we intend to take. A written proposal, a counter-
argument or a measurement is a full contribution.

- **Seal by identity, not by path.** Today a different program launched by a
  sealed one escapes. Anthropic's `sandbox-runtime` fences a dedicated
  account instead. Compare both designs for an assistant that needs the
  microphone and the camera. *Start from:* `crates/waly-seal/src/wfp.rs`.
- **Seal the opened exits per destination.** A gateway of our own, sealed
  except towards a local proxy held by the service, which only accepts the
  host of the exit the user opened. *Start from:*
  `docs/ADR-2026-10-01-lot3-sorties-ouvertes.md`,
  `crates/waly-core/src/exterieur.rs`.
- **Bring Ollama inside the seal.** Seal it by default, open its exit only
  for the time of a download the user asked for.
- **Try to break the seal.** Run the brick on your Windows, behind a VPN or
  an antivirus that installs its own filters, from an unusual install path,
  and report what you see. A bypass is the most valuable report we can get
  (see [SECURITY.md](../SECURITY.md)).
- **Measure a learned turn detector.** Our end-of-turn logic is a heuristic
  on the transcript (`crates/waly-voice/src/endpoint.rs`). Open audio models
  such as Pipecat's Smart Turn exist as small ONNX files and may do better
  on the same ONNX runtime we already load. Bench it on French and report.

### The sharing relay (`waly-relais`)

A tiny std-only HTTP server that stores sealed envelopes
([README](../crates/waly-relais/README.md)).

5. **Per-address rate limiting.** The relay bounds sizes and counts but not
   request rates. Add a simple token bucket per client address. *Touch:*
   `crates/waly-relais/src/lib.rs` (`servir`). *Verify:* a unit test that
   the N+1th request in a window gets `429`.

6. **Dockerfile and a systemd unit.** Make self-hosting a five-minute job,
   with the Caddy snippet from the README. *Verify:* `curl /v1/sante`
   returns `"ok"` through the proxy.

### English mode

Waly was built in French first; an English mode is a top roadmap item.

7. **Inventory of what is French.** List every user-facing string in
   `apps/desktop/ui/index.html`, every prompt in `crates/waly-core/src/prompt.rs`
   and the desktop system prompts, and the French-only speech guards in
   `crates/waly-voice/src/sanitize.rs` and `endpoint.rs`. Deliverable: a
   short design note in `docs/` proposing how to switch language.

8. **Externalise the interface strings.** Move the UI strings of one settings
   pane behind a small dictionary (`fr`, `en`) as a pattern for the rest.
   *Touch:* `apps/desktop/ui/index.html`.

### Core

9. **Linux key vault.** API keys and identities are encrypted with Windows
   DPAPI (`exterieur::proteger`); on Linux the functions return an error.
   Implement a Linux backend (Secret Service, or a `0600` key file as a
   first step). *Touch:* `crates/waly-core/src/exterieur.rs`.

10. **Router test corpus.** `exterieur::router` decides what stays local and
    what may go to an outside model. Add a table of real-looking French and
    English messages with the expected verdict. *Touch:* the tests of
    `crates/waly-core/src/exterieur.rs`.

## With hardware

11. **Measure a full turn on non-reference hardware.** Run a chat turn or
    the voice cascade on a different CPU/GPU/NPU/RAM profile and file the
    numbers with the *Measurement on my machine* issue form. See
    `engines/README.md` for the method. The single most useful contribution
    right now.

12. **Generic OpenAI-compatible backend recipes.** `waly.toml` targets any
    loopback OpenAI-compatible server with automatic 42626 → 11434 fallback.
    Add per-engine setup recipes (llama.cpp, LM Studio) to
    `engines/README.md` and test one end-to-end.

13. **Try an outside model for real.** The outside-model path (Claude,
    Mistral, OpenAI, your own server) is covered by offline end-to-end tests
    but has not been exercised against the real providers. Use your own key,
    report what works and what does not. Never paste a key in an issue.

## Docs & polish

14. **A short GIF of the "verify now" flow.** The README has screenshots; a
    ten-second capture of the seal test (blocked attempt appearing in the
    journal) would say it better.

15. **English mirror drift check.** `README.md` (EN) and `README.fr.md` (FR)
    can drift. Reconcile them (content, not word-for-word).

---

*Filing your own: measurements, logs and portability reports are welcome
even without a linked issue — open one with your machine profile and what
you saw.*
