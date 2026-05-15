// Worker thread: owns the hotkey listener, audio recorder, transcriber, and
// typer. Posts state transitions and live audio level into the shared overlay
// state via Mutex so the Win32 overlay can render in real time.

use crate::config::{Config, models_dir};
use crate::hotkey::{HotkeyEvent, spawn_listener};
use crate::overlay::{OverlayState, SharedOverlay};
use crate::recorder::Recorder;
use crate::transcriber::{Transcriber, ensure_model};
use crate::tray::ControlEvent;
use crate::typer::type_text;
use anyhow::Result;
use std::sync::mpsc::Receiver;
use std::time::Duration;

pub fn worker_loop(
    mut cfg: Config,
    shared: SharedOverlay,
    ctrl_rx: Receiver<ControlEvent>,
) -> Result<()> {
    tracing::info!("loading model: {}", cfg.whisper_model);
    let model_path = ensure_model(&cfg.whisper_model, &models_dir()?)?;
    let mut transcriber = Transcriber::load(&model_path)?;
    tracing::info!("model loaded");

    let events = spawn_listener(&cfg.hotkey)?;
    let mut recorder = Recorder::new();

    tracing::info!("ready — hold [{}] to dictate", cfg.hotkey);

    loop {
        // Drain any pending tray control events. Each iteration of the outer
        // loop runs every ~33ms so this is responsive enough; model reloads
        // block this loop for as long as ensure_model + Transcriber::load
        // take, which is fine because we shouldn't be transcribing during a
        // model swap anyway.
        while let Ok(ev) = ctrl_rx.try_recv() {
            match ev {
                ControlEvent::SwitchModel(name) => {
                    if name == cfg.whisper_model {
                        continue;
                    }
                    if recorder.is_recording() {
                        tracing::warn!("model switch requested mid-recording; dropping audio");
                        let _ = recorder.stop();
                    }
                    tracing::info!("switching model: {} -> {}", cfg.whisper_model, name);
                    set_state(&shared, OverlayState::Processing);
                    match ensure_model(&name, &models_dir()?)
                        .and_then(|p| Transcriber::load(&p))
                    {
                        Ok(t) => {
                            transcriber = t;
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
            }
        }

        if recorder.is_recording() {
            let level = recorder.current_level();
            shared.lock().audio_level = level;
        }

        match events.recv_timeout(Duration::from_millis(33)) {
            Ok(HotkeyEvent::Pressed) => {
                if !recorder.is_recording() {
                    if let Err(e) = recorder.start() {
                        tracing::error!("failed to start recording: {e:#}");
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
                    match transcriber.transcribe(&samples, &cfg.language) {
                        Ok(text) if !text.is_empty() => {
                            tracing::info!("→ {}", text);
                            if let Err(e) = type_text(&text) {
                                tracing::error!("typing failed: {e:#}");
                            }
                        }
                        Ok(_) => tracing::info!("(empty)"),
                        Err(e) => tracing::error!("transcription failed: {e:#}"),
                    }
                    set_state(&shared, OverlayState::Idle);
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
