# Known Issues / Feature Requests

## Cannot cancel an in-progress transcription

After releasing the hotkey, transcription runs to completion with no way to abort. On larger models (`small.en`, `medium.en`) this can take many seconds, during which the user is stuck waiting — they may have already moved on, switched windows, or no longer want the text injected.

**Needed:** a way to cancel a transcription in progress.

Ideas:
- Press Esc (or re-tap the hotkey) while the "processing" overlay is showing to abort.
- Tray menu item: "Cancel current transcription".
- Internally: whisper-rs runs sync; cancellation likely needs an abort flag checked between segments, or running inference on a thread we can detach/drop the result from. Simplest first pass: keep inference running but discard the result if a cancel was requested before typing — saves the user from unwanted text injection even if it doesn't free the CPU immediately.

Repro: set `whisper_model` to `medium.en`, record a few seconds of speech, release. No way to bail out during processing.

## No way to choose the input microphone

The recorder always uses the system default input device (`src/recorder.rs:27-29`). Users often have multiple mics — a laptop's built-in array, a headset, a USB condenser, a webcam mic — and the OS default isn't always the one they want for dictation.

**Needed:** a way to pick which microphone is used.

Ideas:
- Tray menu submenu: "Microphone ▸" listing available input devices (via `cpal::Host::input_devices()`), with the current selection ticked. Selecting one persists to config and re-opens the input stream.
- Config field `input_device: Option<String>` — `None` keeps current behaviour (system default), `Some(name)` matches by device name. Fall back to default if the named device disappears (unplugged headset, etc.) and log a warning.
- Optional later: a "test mic" item that shows a live input level meter so the user can confirm the right device is selected before recording for real.

## No feedback during model switch

When you change the model from the tray, the overlay pops up in `Processing` state (`src/app.rs:48`) while the new model loads — good, the user sees *something* is happening, but there's no indication of *what*. With larger models the load can take 10+ seconds and it looks identical to a transcription that's hung.

**Needed:** a label on the overlay during model load (and arguably during transcription too).

Ideas:
- Add an optional caption string to `OverlayState` / `OverlaySharedState` and render it under the dots — e.g. "Loading base.en…", "Loading medium.en…".
- Or add a distinct `OverlayState::LoadingModel { name: String }` variant so the renderer can pick the text itself and we keep transcription's `Processing` clean.
- While we're there, consider a similar caption for the transcription phase ("Transcribing…") so the two states are visually distinguishable.
