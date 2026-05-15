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
