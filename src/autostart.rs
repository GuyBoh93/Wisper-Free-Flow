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

// Always rewrite the registry value on enable: `auto-launch::is_enabled()` only
// checks an entry exists by name, not that it points to the current exe. If the
// user moves the binary (portable use, upgrade from dev to installer, etc.) the
// old path would otherwise stay registered and autostart would silently break.
pub fn sync(enabled: bool) -> Result<()> {
    let auto = handle()?;
    if enabled {
        auto.enable().context("enabling autostart")?;
        tracing::info!("autostart enabled for current exe");
    } else if auto.is_enabled().unwrap_or(false) {
        auto.disable().context("disabling autostart")?;
        tracing::info!("autostart disabled");
    }
    Ok(())
}
