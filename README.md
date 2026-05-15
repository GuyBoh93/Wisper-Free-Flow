# Whisper FreeFlow

**Free, open-source push-to-talk dictation.** Hold a hotkey, speak, release — text appears at your cursor. Anywhere. Offline.

A no-cost alternative to apps like Wispr Flow. Runs entirely on-device via [whisper.cpp](https://github.com/ggerganov/whisper.cpp), no cloud, no subscription, no telemetry.

---

## Features

- **Push-to-talk** — hold the hotkey while you speak, release to transcribe.
- **Types at your cursor** — works in any text field, any app.
- **Fully offline** — Whisper runs locally. Your audio never leaves the machine.
- **Tiny** — ~3 MB installer. Lives in the system tray.
- **Auto-start on login** (on by default; toggle in config).
- **Live overlay** — small pill at the bottom of the screen shows a waveform while recording and a pulse while transcribing.
- **Cross-platform** — Windows and macOS. (Linux compiles but isn't a primary target yet.)

---

## Install

### Windows

1. Download the latest `whisper-freeflow_x.y.z_x64-setup.exe` from [Releases](https://github.com/GuyBoh93/Wisper-Free-Flow/releases).
2. Run the installer. A tray icon appears.
3. Hold **Shift + Space** and start talking.

### macOS

1. Download the latest `Whisper FreeFlow_x.y.z_aarch64.dmg` (Apple Silicon) or `_x86_64.dmg` (Intel) from Releases.
2. Open the DMG, drag the app to **Applications**.
3. The first time you launch, macOS will ask for **Microphone** access (recording) and **Accessibility** access (typing into other apps). Both are required.
4. Hold **Shift + Space** and start talking.

> On first launch the app downloads the default Whisper model (~150 MB) from Hugging Face. Subsequent launches are instant.

---

## How to use

Hold the hotkey while you talk. Release. The transcribed text appears wherever the cursor is.

A small overlay shows a live waveform while you're recording, then a pulsing indicator while Whisper is transcribing.

That's it.

---

## Configuration

A JSON file at:

| OS      | Path                                                                   |
|---------|------------------------------------------------------------------------|
| Windows | `%APPDATA%\WhisperFreeFlow\config.json`                                |
| macOS   | `~/Library/Application Support/WhisperFreeFlow/config.json`            |
| Linux   | `~/.local/share/WhisperFreeFlow/config.json` (or `$XDG_DATA_HOME`)     |

Defaults:

```json
{
  "hotkey": "shift+space",
  "whisper_model": "base.en-q5_1",
  "whisper_device": "cpu",
  "language": "en",
  "autostart": true
}
```

### Language

ISO-639-1 code (e.g. `en`, `fr`, `de`, `ja`). Use `auto` to let Whisper detect the spoken language on each utterance. **Note:** `.en`-suffixed models are English-only — pair non-English / `auto` with a multilingual model like `base`, `small`, `medium`.

### Hotkey

Format is `mod+mod+...+key`, case-insensitive. Examples:

- `shift+space` (default)
- `ctrl+alt+d`
- `right_alt+space`
- `f9` (no modifier needed)

Supported modifiers: `shift`, `ctrl`, `alt`, `cmd` / `win` / `super`. Add `r` or `right_` prefix for the right-hand modifier. The main key may be a letter, digit, function key (`f1`–`f12`), or a named key (`space`, `tab`, `enter`, `escape`, `home`, `end`, etc.).

### Models

Models are downloaded from `huggingface.co/ggerganov/whisper.cpp` on first use and cached at `<app_data>/models/`. Set `whisper_model` to any of:

| Name              | Size    | Notes                                            |
|-------------------|---------|--------------------------------------------------|
| `tiny.en`         | ~75 MB  | Fastest, lower accuracy.                         |
| `base.en`         | ~150 MB | Good balance.                                    |
| `base.en-q5_1`    | ~60 MB  | **Default.** Quantized base — faster, similar accuracy. |
| `small.en`        | ~500 MB | Better accuracy, slower.                         |
| `medium.en`       | ~1.5 GB | Best accuracy for dictation, noticeably slower.  |

`.en` models are English-only and noticeably faster than multilingual variants. Drop `.en` (e.g. `base`) for any supported language at the cost of ~30% inference speed.

---

## Build from source

### Prerequisites

- **Rust** ≥ 1.85 (Edition 2024) — install via [rustup](https://rustup.rs/).
- **CMake** — whisper.cpp builds from source.
  - Windows: install [CMake](https://cmake.org/download/) and add to PATH.
  - macOS: `brew install cmake`.
  - Linux: `apt install cmake` (or your distro's equivalent).
- **C/C++ toolchain** —
  - Windows: [MSVC Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) with the "Desktop development with C++" workload.
  - macOS: `xcode-select --install`.
  - Linux: `apt install build-essential`.
- **LLVM/libclang** (Windows only) — required by `bindgen` for whisper-rs. Install [LLVM](https://github.com/llvm/llvm-project/releases) and ensure `LIBCLANG_PATH` points at the `bin` directory. `build.bat` will detect a standard install at `C:\Program Files\LLVM`.

### Commands

Windows (`build.bat`):

```bat
build.bat            :: debug build + run
build.bat release    :: optimized release build
build.bat installer  :: build + produce NSIS installer in dist/
build.bat clean      :: cargo clean
```

macOS / Linux (`build.sh`):

```bash
chmod +x build.sh
./build.sh             # debug build + run
./build.sh release     # optimized release build
./build.sh installer   # build + produce .dmg (macOS) / .AppImage (Linux)
./build.sh clean
```

Or use cargo directly:

```bash
cargo run --release
```

The first build will take 5–10 minutes because whisper.cpp is compiled from source. Subsequent builds are incremental.

---

## How it works

```
[ keyboard ]──▶ rdev grab ──▶ hotkey listener ──▶ cpal recorder ─┐
                                                                 │
                              tray-icon ──▶ main thread          │
                                                                 ▼
              enigo (type) ◀── whisper.cpp inference ◀── 16 kHz mono buffer
                                                                 │
                              tiny-skia ──▶ overlay window ◀─────┘
```

- **`recorder.rs`** captures audio via `cpal` from the default input device, then resamples to 16 kHz mono on stop (whisper.cpp's native input).
- **`hotkey.rs`** uses `rdev::grab` to listen globally and *suppress* the hotkey's main key so e.g. holding **Shift + Space** doesn't leak a space character into the focused app.
- **`transcriber.rs`** runs whisper.cpp with greedy sampling, single-segment, no timestamps, with `audio_ctx = 768` (~2× faster than default 1500 for short utterances).
- **`typer.rs`** injects text at the cursor via `enigo` (SendInput on Windows, CGEvent on macOS, XTest on Linux).
- **`overlay.rs`** draws a click-through, always-on-top pill at the bottom of the screen via a layered Win32 window on Windows. Pixels are rasterised by `tiny-skia` and blitted with `UpdateLayeredWindow`. (macOS overlay is a stub for now.)
- **`tray.rs`** owns the main thread, runs the platform event loop, and exits when the user clicks **Quit**.

---

## Privacy

- Audio is captured to an in-memory buffer, transcribed, and discarded. Nothing is written to disk except the model cache.
- No network requests except for the **one-time model download** from Hugging Face on first run.
- No telemetry, analytics, or crash reporting.

---

## Status & roadmap

This is a young project. The hot path (Windows: hotkey → record → transcribe → type) is solid. Known gaps:

- [ ] macOS overlay window (currently no visual feedback on Mac).
- [ ] macOS tray menu visual polish (icon currently procedurally generated).
- [ ] Metal/CUDA inference backends (CPU only today).
- [ ] Configurable language / multilingual model selection in UI.
- [ ] Linux: works but untested.
- [ ] Localised punctuation models, custom vocab.

Contributions welcome — open an issue or PR on GitHub.

---

## License

[MIT](LICENSE). Free for personal and commercial use.

Whisper model weights are licensed by OpenAI under MIT; whisper.cpp itself is MIT-licensed by Georgi Gerganov.

---

## Acknowledgements

- [whisper.cpp](https://github.com/ggerganov/whisper.cpp) by Georgi Gerganov.
- [OpenAI Whisper](https://github.com/openai/whisper) for the underlying model.
- [`rdev`](https://crates.io/crates/rdev), [`cpal`](https://crates.io/crates/cpal), [`enigo`](https://crates.io/crates/enigo), [`tray-icon`](https://crates.io/crates/tray-icon), [`tiny-skia`](https://crates.io/crates/tiny-skia) — the Rust crates that make this app possible.
