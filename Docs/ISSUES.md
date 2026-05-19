# Known Issues / Feature Requests

## Cannot cancel an in-progress transcription — FIXED 2026-05-19

After releasing the hotkey, transcription used to block the worker loop with no way to bail out. On `small.en` / `medium.en` this could mean many seconds of forced wait, after which unwanted text would land at the cursor even if the user had already moved on.

Fix:
1. **Off-thread inference.** `app::worker_loop` now spawns a short-lived thread for each transcribe call and holds a `Pending { rx, cancel }` (`src/app.rs`). The worker keeps polling hotkey + tray events while inference runs, so the UI stays responsive.
2. **Re-press to cancel.** Pressing the hotkey again while the overlay is in `Processing` state sets the `cancel` AtomicBool. When the inference thread eventually finishes, the worker checks the flag and drops the text instead of typing it. The cancel gesture is checked before the start-recording path so it doesn't immediately start a new recording on top of the cancel.
3. **Tray menu item.** Added "Cancel transcription" (`src/tray.rs`) which sends `ControlEvent::CancelTranscription` — the worker handles it the same way as a re-press. Always enabled; ignored when nothing is pending.
4. **Model switch.** Switching model mid-transcription now also flags the pending result as cancelled (`SwitchModel` handler) so the old transcriber's output doesn't get typed after a swap.

Caveat: whisper.cpp doesn't expose mid-segment abort in `whisper-rs` 0.16, so the CPU keeps spinning on the discarded inference until it completes naturally. The user-visible problem (unwanted text injection + frozen UI) is gone; a future enhancement could plumb whisper's abort callback through for true mid-flight cancellation.

Repro: set `whisper_model` to `medium.en`, hold the hotkey for a few seconds, release. While the dots are showing, tap the hotkey once — log shows `"cancel requested"`, overlay closes, no text is typed.

## No way to choose the input microphone — FIXED 2026-05-19

Recorder used to be hard-wired to the system default input device, which on multi-mic setups (built-in array + headset + USB condenser + webcam) was rarely the one the user actually wanted for dictation.

Fix:
1. **Config field.** New `input_device: Option<String>` in `src/config.rs`; `None` means "use system default" (preserves prior behaviour), `Some(name)` matches by cpal device name.
2. **Tray submenu.** "Microphone ▸" submenu (`src/tray.rs`) lists "System default" plus each detected input device, with the current selection ticked. Clicking one sends `ControlEvent::SetInputDevice(Option<String>)` to the worker, which updates the recorder's `preferred_device` and persists the config. Built once at startup — devices that arrive/leave later won't show up until the next launch.
3. **Graceful fallback.** `recorder::pick_device()` tries the named device first and falls back to the system default with a warning if it's missing (unplugged headset, disabled in OS settings). The tray menu also re-ticks "System default" at startup if the saved device name can't be found.
4. **Enumeration helper.** New `recorder::list_input_devices()` used by the tray to build the submenu; tolerates per-device query failures rather than aborting the whole list.

Repro: pick a non-default mic from the tray submenu. Hold the hotkey — log shows the selected device name and audio captures from it. Unplug that device, hold the hotkey — log shows the fallback warning and recording uses the system default.

## No way to choose the input microphone

The recorder always uses the system default input device (`src/recorder.rs:27-29`). Users often have multiple mics — a laptop's built-in array, a headset, a USB condenser, a webcam mic — and the OS default isn't always the one they want for dictation.

**Needed:** a way to pick which microphone is used.

Ideas:
- Tray menu submenu: "Microphone ▸" listing available input devices (via `cpal::Host::input_devices()`), with the current selection ticked. Selecting one persists to config and re-opens the input stream.
- Config field `input_device: Option<String>` — `None` keeps current behaviour (system default), `Some(name)` matches by device name. Fall back to default if the named device disappears (unplugged headset, etc.) and log a warning.
- Optional later: a "test mic" item that shows a live input level meter so the user can confirm the right device is selected before recording for real.

## Transcription is too slow, and there's no sense of how long it will take

Even `tiny.en` feels slow on a low-spec ("potato") PC, and the issue compounds with longer recordings — a 30-second readout takes noticeably longer than a 5-second one, but the overlay gives no indication of progress or remaining time. You just see the dots and wait, with no way to tell if it's nearly done or barely started.

**Needed:** (a) faster inference on weak hardware, and (b) an accurate progress indicator so the user can see how long is left.

Ideas:
- **Progress bar in the overlay:** whisper.cpp processes audio in chunks/segments and exposes a progress callback (`whisper_full_params.progress_callback` / `new_segment_callback`). Wire that through `whisper-rs` to update an `OverlayState::Processing { progress: f32 }` variant, and render an actual progress bar instead of indeterminate dots. Progress should be a real fraction of audio processed, not a fake timer.
- **Faster models / backends:**
  - Try `ggml-tiny.en-q5_1.bin` or other quantised variants — smaller and faster than the default fp16 `tiny.en`, often with minimal accuracy loss on clean dictation.
  - Look at distilled / turbo variants (e.g. `distil-whisper` ggml builds) if compatible with whisper.cpp.
  - Build with hardware acceleration where available: Vulkan/OpenCL on Windows for GPUs that don't have CUDA, BLAS/AVX feature flags on CPU. Currently `whisper_device: "cpu"` is the only path exercised.
  - Consider exposing a "speed vs accuracy" preset in the tray that picks model + quantisation + thread count together, rather than making the user understand all three knobs.
- **Tune whisper params for short dictation:** `n_threads` (set to physical core count), `no_context = true`, `single_segment = true` for short utterances — all reduce latency without changing the model.

Repro: on a low-spec machine, hold the hotkey for 20–30 seconds reading a paragraph, release. The wait before text appears feels open-ended because the overlay shows no progress.

## No feedback during model switch

When you change the model from the tray, the overlay pops up in `Processing` state (`src/app.rs:48`) while the new model loads — good, the user sees *something* is happening, but there's no indication of *what*. With larger models the load can take 10+ seconds and it looks identical to a transcription that's hung.

**Needed:** a label on the overlay during model load (and arguably during transcription too).

Ideas:
- Add an optional caption string to `OverlayState` / `OverlaySharedState` and render it under the dots — e.g. "Loading base.en…", "Loading medium.en…".
- Or add a distinct `OverlayState::LoadingModel { name: String }` variant so the renderer can pick the text itself and we keep transcription's `Processing` clean.
- While we're there, consider a similar caption for the transcription phase ("Transcribing…") so the two states are visually distinguishable.

## App had no graceful handling for "no microphone present" — FIXED 2026-05-19

Symptom: running on a machine with no input device (no built-in mic, no headset, mic disabled in Windows privacy settings) gave no feedback — pressing the hotkey silently logged an error and nothing happened. Reported as "crashes instantly" on a mic-less test PC.

Fix:
1. **Startup probe.** `app::worker_loop` now calls `recorder::has_input_device()` on entry and, if it returns false, flashes a new `OverlayState::NoMic` pill for 2 seconds so the user sees the problem immediately at launch instead of finding out later (`src/app.rs`).
2. **Hotkey-press guard.** Before calling `Recorder::start()`, the worker re-checks `has_input_device()`; if absent (mic unplugged after launch), it shows the NoMic indicator and skips the cpal stream build. If `Recorder::start()` itself errors, it falls through to the same indicator. The 2-second hold (`NO_MIC_HOLD`) auto-clears back to Idle.
3. **NoMic glyph.** New `draw_no_mic` in `src/overlay.rs` renders a microphone capsule + U-stand with a red diagonal slash — clear at a glance, no text rendering required.

Repro: disable the default recording device in `mmsys.cpl` (Recording tab → right-click → Disable), launch the app — the slashed-mic pill appears for 2 s. Press the hotkey — it appears again. Re-enable the mic and dictation works normally.

## Autostart didn't survive moving / reinstalling the exe — FIXED 2026-05-15

Symptom: app failed to launch at login after install. Root cause was two-fold:

1. **Stale registry path.** `auto-launch::is_enabled()` only checks whether an entry exists *by name*, not whether the path matches the current exe. So if the binary moved (dev build → installed location → portable copy), `sync(true)` skipped the rewrite and HKCU\Run kept pointing at the old (sometimes deleted) path. Fixed in `src/autostart.rs:18-22` — `enable()` is now called unconditionally when `enabled=true`, so the registry path is always refreshed to `current_exe()` on launch. This is what makes "drop the exe anywhere, run it once" portable autostart work.
2. **No way to recover from `autostart: false` in config.** The tray menu had no toggle, so once the config field was `false` (whether through stale state or manual edit) the user was stuck. Added a "Start at login" check item to the tray menu (`src/tray.rs`) wired through a new `ControlEvent::SetAutostart` to the worker, which calls `autostart::sync` and persists `cfg.autostart`.
