// Register / unregister the app to launch at user login.
// Windows: HKCU\...\Run registry entry. macOS: LaunchAgent plist in ~/Library.

use anyhow::{Context, Result};
use auto_launch::AutoLaunch;

const APP_NAME: &str = "Whisper FreeFlow";

fn handle() -> Result<AutoLaunch> {
    let exe = std::env::current_exe().context("getting current exe path")?;
    let exe_str = exe.to_str().context("exe path not UTF-8")?;
    Ok(AutoLaunch::new(APP_NAME, exe_str, &[] as &[&str]))
}

pub fn sync(enabled: bool) -> Result<()> {
    let auto = handle()?;
    let currently = auto.is_enabled().unwrap_or(false);
    match (enabled, currently) {
        (true, false) => {
            auto.enable().context("enabling autostart")?;
            tracing::info!("autostart enabled");
        }
        (false, true) => {
            auto.disable().context("disabling autostart")?;
            tracing::info!("autostart disabled");
        }
        _ => {}
    }
    Ok(())
}
