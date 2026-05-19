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

## Transcription is too slow, and there's no sense of how long it will take — PARTIALLY FIXED 2026-05-19

The progress-indicator half is now done; the raw-speed half remains open for hardware-acceleration work.

Fixed:
1. **Real progress bar in the overlay.** Wired whisper-rs's `set_progress_callback_safe` through `Transcriber::transcribe` (`src/transcriber.rs`) up to a new `progress: f32` field on `OverlaySharedInner` (`src/overlay.rs`). New `draw_progress_bar` renders a thin filled track at the bottom of the pill during `Processing`, so the user can see how far inference has got. Bar is reset to 0 on every Idle transition and at the start of each new transcription so the next run starts clean.
2. **True mid-segment cancel.** While we were in the callback area, also wired `set_abort_callback_safe` — the cancel `AtomicBool` from issue #1 is now also read by whisper.cpp itself, so a re-press of the hotkey aborts inference instead of just discarding the result. The CPU spin issue noted in the issue #1 fix is now gone for real. Cancel also clears `Pending` immediately (instead of waiting for the orphan thread) so the user can start a new recording on the very next hotkey press; the orphan still has its own Arc clone of the cancel flag so it aborts cleanly.
3. **Param tweak: `set_no_context(true)`.** Disables prior-utterance conditioning, which is the right default for push-to-talk dictation (each press is independent). Small but free latency win.
4. **Default model is already quantised.** `Config::default()` sets `whisper_model = "base.en-q5_1"` and the tray exposes both `base.en` and `base.en-q5_1` for users to compare.
5. **Physical core count for `n_threads`.** Switched from `std::thread::available_parallelism()` (logical cores) to `num_cpus::get_physical()` (physical). Hyperthread siblings share an AVX execution unit and contend during whisper's matmul, so on a typical 8-core/16-thread CPU 8 threads beats 16. Typical wins: 10–20% lower latency on Intel/AMD parts; no change on CPUs without HT.
6. **Process priority boost during inference.** New `src/priority.rs` raises the whole process to `ABOVE_NORMAL_PRIORITY_CLASS` while a transcription is running and drops it back to `NORMAL` when the result is collected (success or cancel). Process-level rather than thread-level because whisper.cpp manages its own thread pool internally. Windows-only; macOS/Linux get a no-op shim.

Still open:
- **Hardware acceleration.** `whisper_device` is honoured nowhere — the build currently only exercises the CPU path. Adding feature-gated `cuda` / `vulkan` / `metal` builds (or just shipping a CUDA-enabled installer alongside the CPU one) is the biggest remaining lever for low-spec or GPU-equipped machines.
- **Distilled / turbo models.** `distil-whisper` ggml builds aren't in the tray submenu; worth evaluating quality on dictation before promoting.
- **"Speed vs accuracy" preset.** Tray currently exposes model and (indirectly) thread count as separate concepts. A single preset that bundles model + quantisation + threads would be friendlier than asking users to understand all three knobs.

Repro for the fixed parts: hold the hotkey for 20 seconds reading a paragraph, release. The overlay now shows a progress bar filling left-to-right as whisper works through the audio. Tap the hotkey while it's still filling — inference aborts immediately (logs `cancel requested`), overlay closes, no further CPU spend.

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
