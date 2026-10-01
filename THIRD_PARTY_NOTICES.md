# Third-party notices

Waly's own code is [MIT](LICENSE). Waly runs on third-party models, voices
and engines that keep their own licenses. **None of the model weights below
are stored in this repository or bundled in the installer** — they are
downloaded by the user into `engines/models/` (see `engines/README.md`).
The only third-party media committed to git are the two reference voices in
`engines/voices-fr/`.

Licenses checked against the upstream pages on 2026-09-10.

## Committed to this repository

| File | Source | License |
|---|---|---|
| `engines/voices-fr/fabien.wav` | [kyutai/tts-voices](https://huggingface.co/kyutai/tts-voices) — `unmute-prod-website/fabieng-enhanced-v2.wav` | **CC0** (Kyutai's own recordings) |
| `engines/voices-fr/developpeuse.wav` | [kyutai/tts-voices](https://huggingface.co/kyutai/tts-voices) — `unmute-prod-website/developpeuse-3.wav` | **CC0** (Kyutai's own recordings) |

## Bundled in the installer

| Component | License |
|---|---|
| `WebView2Loader.dll` (Microsoft WebView2 SDK) | Microsoft WebView2 SDK license terms (redistributable loader) |

## Downloaded by the user (not redistributed)

| Component | Used for | License | Notes |
|---|---|---|---|
| Qwen3-VL-4B-Instruct (`qwen3vl-it:4b`) | default brain, text + vision | Apache 2.0 | |
| Qwen3-4B-Instruct-2507 | fallback brain (Ollama) | Apache 2.0 | |
| [Pocket TTS](https://huggingface.co/kyutai/pocket-tts) `french_24l` (Kyutai), via the [ONNX export](https://huggingface.co/KevinAHM/pocket-tts-onnx) | French voice | Model: **CC BY 4.0**; ONNX export code: Apache 2.0 | Kyutai's terms **prohibit voice impersonation or cloning without explicit and lawful consent**, and presenting generated audio as authentic recordings. |
| Piper `fr_FR-upmc-medium` | fallback voice | **CC BY-SA 4.0** (dataset [upmc-pierre-data](https://github.com/marytts/upmc-pierre-data)) | |
| [Parakeet-TDT 0.6B v3](https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3) (NVIDIA), int8 | speech-to-text | **CC BY 4.0** | commercial use allowed with attribution |
| [Silero VAD](https://github.com/snakers4/silero-vad) v5 | voice activity detection | MIT | |
| multilingual-e5-small, int8 | memory embeddings | MIT | |
| [YuNet](https://github.com/opencv/opencv_zoo/tree/main/models/face_detection_yunet) `2023mar` | camera presence | MIT | |
| PaddleOCR PP-OCRv4 det / PP-OCRv3 en rec (RapidOCR ONNX) | screen OCR | Apache 2.0 | |
| [openWakeWord](https://github.com/dscripka/openWakeWord) `melspectrogram.onnx`, `embedding_model.onnx` | wake-word features | Code Apache 2.0; **pre-trained models CC BY-NC-SA 4.0** | ⚠ **Non-commercial.** See below. |
| ONNX Runtime (`onnxruntime.dll`, Microsoft-signed) | inference runtime | MIT | |
| [FastFlowLM](https://github.com/FastFlowLM/FastFlowLM) | NPU inference engine | CLI/orchestration: MIT; NPU kernels: free for any use, including commercial | Attribution requested: **Powered by FastFlowLM**. |
| [Ollama](https://github.com/ollama/ollama) | fallback inference engine | MIT | |
| [MCP filesystem server](https://github.com/modelcontextprotocol/servers) `@modelcontextprotocol/server-filesystem` (optional, installed with `npm --prefix` into `engines/mcp/`) | read-only document tools via MCP | Apache 2.0 (new contributions) / MIT (existing code) | runs outside the network seal (see `docs/ADR-2026-09-10-client-mcp-stdio.md`) |

### ⚠ Wake word: non-commercial feature models

The wake word "Waly" is a small classifier on top of openWakeWord's shared
feature models, which upstream publishes under **CC BY-NC-SA 4.0**. Until
this is replaced, **the wake word feature is non-commercial only**.
Remediation paths (tracked in the roadmap): re-export the Google speech
embedding from its original Apache-2.0 TFHub release, and compute the mel
front-end in Rust instead of `melspectrogram.onnx`.

The trained wake classifier itself (`waly_wake.onnx`) is **not published**:
the production version was fine-tuned on real recordings of one person's
voice (biometric data).

## Rust and JavaScript dependencies

Crate dependencies keep their own licenses (listed in `Cargo.lock`; mostly
MIT / Apache-2.0). A generated per-crate license report will ship with the
first binary release.
