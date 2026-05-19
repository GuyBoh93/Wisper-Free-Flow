// Worker thread: owns the hotkey listener, audio recorder, transcriber, and
// typer. Posts state transitions and live audio level into the shared overlay
// state via Mutex so the Win32 overlay can render in real time.
//
// Transcription runs on a short-lived helper thread so the worker stays
// responsive during inference — that's what makes the "re-press to cancel"
// behaviour possible. We can't *abort* whisper.cpp mid-segment, so cancel
// just sets a flag that suppresses text injection when the inference
// eventually completes (the CPU keeps spinning in the background until
// then). Good enough to stop unwanted text without changing whisper.cpp.

use crate::autostart;
use crate::config::{Config, models_dir};
use crate::hotkey::{HotkeyEvent, spawn_listener};
use crate::overlay::{OverlayState, SharedOverlay};
use crate::recorder::{Recorder, has_input_device};
use crate::transcriber::{Transcriber, ensure_model};
use crate::tray::ControlEvent;
use crate::typer::type_text;
use anyhow::Result;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::{Duration, Instant};

// How long the "no mic" overlay stays up after a failed start before
// reverting to Idle. Long enough to be noticed, short enough to disappear
// before the user tries again.
const NO_MIC_HOLD: Duration = Duration::from_secs(2);

/// In-flight transcription. The worker polls `rx`; the user can set
/// `cancel` to discard the result on completion. The Arc is held by both
/// the worker thread and the helper thread doing the inference.
struct Pending {
    rx: std::sync::mpsc::Receiver<Result<String>>,
    cancel: Arc<AtomicBool>,
}

pub fn worker_loop(
    mut cfg: Config,
    shared: SharedOverlay,
    ctrl_rx: Receiver<ControlEvent>,
) -> Result<()> {
    tracing::info!("loading model: {}", cfg.whisper_model);
    let model_path = ensure_model(&cfg.whisper_model, &models_dir()?)?;
    // Arc so the inference thread can hold its own reference for the
    // duration of the call. Replaced when the user switches model.
    let mut transcriber = Arc::new(Transcriber::load(&model_path)?);
    tracing::info!("model loaded");

    let events = spawn_listener(&cfg.hotkey)?;
    let mut recorder = Recorder::new(cfg.input_device.clone());
    let mut no_mic_until: Option<Instant> = None;
    let mut pending: Option<Pending> = None;

    // One-shot startup probe so the user sees the warning immediately on a
    // mic-less machine instead of finding out the first time they hit the
    // hotkey. The overlay auto-hides after NO_MIC_HOLD.
    if !has_input_device() {
        tracing::warn!("no input device detected at startup");
        set_state(&shared, OverlayState::NoMic);
        no_mic_until = Some(Instant::now() + NO_MIC_HOLD);
    }

    tracing::info!("ready — hold [{}] to dictate", cfg.hotkey);

    loop {
        // Drop the NoMic indicator once its hold window has elapsed.
        if let Some(deadline) = no_mic_until {
            if Instant::now() >= deadline {
                no_mic_until = None;
                if shared.lock().state == OverlayState::NoMic {
                    set_state(&shared, OverlayState::Idle);
                }
            }
        }

        // Drain any pending tray control events. Each iteration of the outer
        // loop runs every ~33ms so this is responsive enough; model reloads
        // block this loop for as long as ensure_model + Transcriber::load
        // take, which is fine because we shouldn't be transcribing during a
        // model swap anyway.
        while let Ok(ev) = ctrl_rx.try_recv() {
            match ev {
                ControlEvent::SetAutostart(enabled) => {
                    if cfg.autostart == enabled {
                        continue;
                    }
                    cfg.autostart = enabled;
                    if let Err(e) = autostart::sync(enabled) {
                        tracing::error!("autostart sync failed: {e:#}");
                    }
                    if let Err(e) = cfg.save() {
                        tracing::error!("saving config: {e:#}");
                    }
                }
                ControlEvent::SwitchModel(name) => {
                    if name == cfg.whisper_model {
                        continue;
                    }
                    if recorder.is_recording() {
                        tracing::warn!("model switch requested mid-recording; dropping audio");
                        let _ = recorder.stop();
                    }
                    if let Some(p) = pending.take() {
                        tracing::warn!("model switch requested mid-transcription; cancelling");
                        p.cancel.store(true, Ordering::Release);
                    }
                    tracing::info!("switching model: {} -> {}", cfg.whisper_model, name);
                    set_state(&shared, OverlayState::Processing);
                    match ensure_model(&name, &models_dir()?).and_then(|p| Transcriber::load(&p)) {
                        Ok(t) => {
                            transcriber = Arc::new(t);
                            cfg.whisper_model = name;
                            if let Err(e) = cfg.save() {
                                tracing::error!("saving config: {e:#}");
                            }
                            tracing::info!("model loaded");
                        }
                        Err(e) => tracing::error!("model switch failed: {e:#}"),
                    }
                    set_state(&shared, OverlayState::Idle);
                }
                ControlEvent::SetInputDevice(name) => {
                    if cfg.input_device == name {
                        continue;
                    }
                    let label = name.as_deref().unwrap_or("(system default)");
                    tracing::info!("switching input device -> {label}");
                    cfg.input_device = name.clone();
                    recorder.set_preferred_device(name);
                    if let Err(e) = cfg.save() {
                        tracing::error!("saving config: {e:#}");
                    }
                }
                ControlEvent::CancelTranscription => {
                    if let Some(p) = pending.as_ref() {
                        tracing::info!("cancel requested — discarding pending transcription");
                        p.cancel.store(true, Ordering::Release);
                        set_state(&shared, OverlayState::Idle);
                    }
                }
            }
        }

        // Poll the in-flight transcription, if any. Inference runs off-thread
        // so this is non-blocking; we only act when the result is ready.
        if let Some(p) = pending.as_ref() {
            match p.rx.try_recv() {
                Ok(result) => {
                    let cancelled = p.cancel.load(Ordering::Acquire);
                    pending = None;
                    if cancelled {
                        tracing::info!("transcription cancelled — result discarded");
                    } else {
                        match result {
                            Ok(text) if !text.is_empty() => {
                                tracing::info!("→ {}", text);
                                if let Err(e) = type_text(&text) {
                                    tracing::error!("typing failed: {e:#}");
                                }
                            }
                            Ok(_) => tracing::info!("(empty)"),
                            Err(e) => tracing::error!("transcription failed: {e:#}"),
                        }
                    }
                    set_state(&shared, OverlayState::Idle);
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    tracing::error!("transcription thread vanished without result");
                    pending = None;
                    set_state(&shared, OverlayState::Idle);
                }
            }
        }

        if recorder.is_recording() {
            let level = recorder.current_level();
            shared.lock().audio_level = level;
        }

        match events.recv_timeout(Duration::from_millis(33)) {
            Ok(HotkeyEvent::Pressed) => {
                // Re-pressing the hotkey while a transcription is pending is
                // the "cancel" gesture. Handled before the start-recording
                // path so it doesn't immediately start a new recording on
                // top of the cancel.
                if let Some(p) = pending.as_ref() {
                    tracing::info!("hotkey re-pressed during processing — cancelling");
                    p.cancel.store(true, Ordering::Release);
                    set_state(&shared, OverlayState::Idle);
                    continue;
                }
                if !recorder.is_recording() {
                    // Fast path: no input device at all. Skip cpal entirely so
                    // we don't show "recording" for a stream that can never
                    // produce samples.
                    if !has_input_device() {
                        tracing::warn!("hotkey pressed with no input device available");
                        set_state(&shared, OverlayState::NoMic);
                        no_mic_until = Some(Instant::now() + NO_MIC_HOLD);
                        continue;
                    }
                    if let Err(e) = recorder.start() {
                        tracing::error!("failed to start recording: {e:#}");
                        set_state(&shared, OverlayState::NoMic);
                        no_mic_until = Some(Instant::now() + NO_MIC_HOLD);
                        continue;
                    }
                    tracing::info!("recording…");
                    set_state(&shared, OverlayState::Recording);
                }
            }
            Ok(HotkeyEvent::Released) => {
                if recorder.is_recording() {
                    let audio = match recorder.stop() {
                        Ok(a) => a,
                        Err(e) => {
                            tracing::error!("failed to stop recording: {e:#}");
                            set_state(&shared, OverlayState::Idle);
                            continue;
                        }
                    };
                    set_state(&shared, OverlayState::Processing);
                    let secs = audio.duration_secs();
                    tracing::info!("captured {:.2}s — transcribing…", secs);
                    if secs < 0.2 {
                        tracing::info!("too short, skipping");
                        set_state(&shared, OverlayState::Idle);
                        continue;
                    }
                    let samples = audio.resample_to_16k();
                    let lang = cfg.language.clone();
                    let trans = Arc::clone(&transcriber);
                    let cancel = Arc::new(AtomicBool::new(false));
                    let (tx, rx) = channel();
                    std::thread::spawn(move || {
                        let result = trans.transcribe(&samples, &lang);
                        // Receiver may be gone if the worker has moved on
                        // (model switch, shutdown) — drop the result silently.
                        let _ = tx.send(result);
                    });
                    pending = Some(Pending { rx, cancel });
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                tracing::error!("hotkey listener disconnected — exiting worker");
                break;
            }
        }
    }

    Ok(())
}

fn set_state(shared: &SharedOverlay, s: OverlayState) {
    let mut inner = shared.lock();
    inner.state = s;
    if s == OverlayState::Idle {
        inner.audio_level = 0.0;
    }
}
