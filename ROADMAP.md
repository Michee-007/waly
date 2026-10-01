# Roadmap

Waly's phases R0–R6b (voice cascade, memory, tools, desktop, vision, screen,
network seal, wake word) are built and measured — see the ADR, RFC and PLAN documents in `docs/` for
the design record. What follows is what we believe matters *now*, given the
state of local AI in late 2026. Ordered by intent, not by promise.

## Now — priorities set by the author (2026-09-11)

A. **Publish.** Waly goes open source to share and demonstrate the work:
   a fresh public repository (no personal biometrics in history), the build
   log, honest measurements and limits.
B. **The screen agent** — where Waly can lead. *Code delivered 2026-09-11,
   field test pending* (`docs/ADR-2026-09-11-ecran-lu-en-texte.md`): the
   screen is read as TEXT through Windows UI Automation (25-450 ms, no
   pixels), actions go through accessibility patterns without moving the
   mouse, each one approved under a human-readable label; "watch me"
   sessions become learned skills.
   - *see your screen live and help* (voice, OCR-first, fresh capture) —
     make it truly continuous and alive, without re-polluting answers;
   - *do it under your eyes* — supervised UI automation (Windows UI
     Automation first), every consequential action approved, parity with
     what other agents already ship;
   - *learn by watching you* — an explicit "watch me" session records your
     steps as text (never pixels, never always-on), distilled into a
     learned skill Waly can replay under supervision.
C. **Huis clos for any agent** — the network seal as a standalone brick
   able to seal any local agent, while Waly itself stays a solid platform.

D. **Exits you open, one by one — delivered 2026-10-01**
   (`docs/ADR-2026-10-01-lot3-sorties-ouvertes.md`): deep reflection, model
   downloads through Ollama, outside models with your own key + a local-first
   router, a Telegram bridge, and conversation sharing between two Waly
   installs through a self-hostable relay. All covered by offline end-to-end
   tests; **field test against real providers pending**. Still to do: seal
   the gateway process per destination (WFP address/port filters — needs a
   service update), Linux key vault, Matrix bridge, calls between two Waly.

## Before wide publication

1. **Re-bench the French voice on the distilled Pocket TTS model — when it
   ships.** The current voice runs on the undistilled `french_24l` preview;
   as of Pocket TTS v3.1.0 (September 2026) Kyutai has still not released a
   distilled French model ("more painful than anticipated due to the data
   quality"). A distilled model should close most of the "voice is slow" gap;
   the native pipeline (`pocket.rs`) can be rewired to it in about an hour.
2. ✅ **`waly.toml` configuration + generic OpenAI-compatible backend —
   done.** One optional file (`data\waly.toml`, see `waly.toml.example`)
   declares the local server port, model, user name and database path;
   environment variables still win. Any OpenAI-compatible loopback server
   (FastFlowLM, Ollama, llama.cpp, LM Studio) is a first-class target, with
   automatic fallback 42626 → 11434. **Adaptive vision** too: Waly probes
   what the brain can do; a text-only brain gets a declared vision model
   (`modele_vision`) that describes images to it, or an honest "I can't see"
   (`docs/ADR-2026-09-10-vision-adaptative.md`). And a **first-run model
   recommendation**: Waly detects RAM, GPU, NPU and Smart App Control,
   compares with installed Ollama models and proposes a brain + vision
   model with the `ollama pull` commands (status-bar model name, or
   `waly materiel`). Next: per-engine recipes (llama.cpp, LM Studio).
3. **Public demo material** (GIF, screenshots) and good-first-issues
   (curated in [docs/GOOD-FIRST-ISSUES.md](docs/GOOD-FIRST-ISSUES.md)).
4. **Lift the wake word's non-commercial dependency.** openWakeWord's shared
   feature models are CC BY-NC-SA 4.0 (see `THIRD_PARTY_NOTICES.md`).
   Re-export the Google speech embedding from its Apache-2.0 TFHub release,
   compute the mel front-end in Rust, and ship a publishable wake model
   (synthetic + consenting donors — never a single person's biometrics).

## Next

5. ✅ **Local MCP client (stdio only) — v1 done.** Community tools plug in
   through the Model Context Protocol: declare servers under `[mcp.<name>]`
   in `waly.toml` (desktop and CLI). Stdio only — Waly opens no port. MCP
   servers run *outside* the network seal, so every MCP tool call requires
   human approval unless the server is declared `confiance = "lecture"`
   (`docs/ADR-2026-09-10-client-mcp-stdio.md`). Next: voice process,
   privacy-panel listing of active servers, contextual tool selection.
6. **Re-bench FastFlowLM 1.0** (now under the AMD ROCm umbrella, Windows and
   Linux): TTFT, conversation cache behavior, model catalog. Several pitfalls
   pinned in our docs date from v0.9.43 and may be obsolete.
7. **English mode** — i18n of prompts and language guards (the product is
   French-first; English must be first-class for the community).
8. **Bench an audio-native brain** (e.g. Gemma 4 E4B audio input) against
   the current STT→LLM cascade: an audio-in LLM folds transcription, intent
   and tone into one pass and removes a whole class of short-utterance STT
   workarounds.
9. ✅ **Native Office documents — done.** File hands are in-tree (list,
   read, search, create/modify real files, every write behind a human
   approval, clickable file cards and Carnet with open-in-Explorer), and
   Waly now generates real **.docx, .xlsx, .pptx and .pdf** from markdown,
   in pure Rust with no new native dependency — verified by opening them
   in Word, Excel, PowerPoint and the Windows PDF engine
   (`lab/documents-banc/`). Next: reading Office files back (text
   extraction), richer layouts (images, charts).

## Later

- **Whole-disk file mapping** — a local semantic index of the user's files
  ("find the contract I wrote last spring"), same privacy rules as the
  visual journal: descriptions and paths, never content exfiltrated.
- **More opt-in online capabilities** — web search, account connections
  (outside models and messaging are in, see D above). The seal stays the
  default state; going online is a deliberate, per-capability activation the
  user sees — never a default.
- **Linux port — in progress** (`docs/PLAN-2026-09-11-R-L-portage-linux.md`).
  Done: portable paths (`WALY_HOME`, XDG), native library names, Linux
  hardware detection. Next: CLI and voice on a real Linux, desktop
  AppImage, V4L2 camera and X11 capture, then the network seal
  (cgroup v2 + nftables). macOS after.
- **Supervised computer use** (browser, applications) — the file hands came
  first; UI-level hands follow when they can be done safely.

## Non-goals

- Ambient always-on recording or persistent ambient memory (see the privacy
  model — this is a deliberate refusal, not a missing feature).
- Telemetry of any kind.
