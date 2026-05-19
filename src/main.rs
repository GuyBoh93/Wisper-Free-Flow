// Whisper FreeFlow — open-source push-to-talk dictation.
//
// Orchestration: spawn the Win32 overlay thread, spawn the audio/hotkey/whisper
// worker thread, then run the tray-icon message pump on the main thread until
// the user clicks Quit.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod autostart;
mod config;
mod hotkey;
mod overlay;
mod recorder;
mod transcriber;
mod tray;
mod typer;

use anyhow::Result;
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    tracing::info!("whisper-freeflow {}", env!("CARGO_PKG_VERSION"));

    let cfg = config::Config::load_or_default()?;
    tracing::info!("config: hotkey={} model={}", cfg.hotkey, cfg.whisper_model);

    if let Err(e) = autostart::sync(cfg.autostart) {
        tracing::warn!("autostart sync failed: {e:#}");
    }

    let shared = overlay::new_shared();
    overlay::spawn(shared.clone());

    let (ctrl_tx, ctrl_rx) = std::sync::mpsc::channel();
    let initial_model = cfg.whisper_model.clone();
    let initial_autostart = cfg.autostart;
    let initial_mic = cfg.input_device.clone();
    let worker_cfg = cfg.clone();
    let worker_shared = shared.clone();
    std::thread::spawn(move || {
        if let Err(e) = app::worker_loop(worker_cfg, worker_shared, ctrl_rx) {
            tracing::error!("worker crashed: {e:#}");
        }
    });

    tray::run_until_quit(initial_model, initial_autostart, initial_mic, ctrl_tx)
}
