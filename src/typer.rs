// Text injection at the cursor via enigo. Cross-platform: SendInput on Windows,
// CGEvent on macOS, XTest on Linux.

use anyhow::{Context, Result};
use enigo::{Enigo, Keyboard, Settings};

pub fn type_text(text: &str) -> Result<()> {
    if text.is_empty() {
        return Ok(());
    }
    let mut enigo = Enigo::new(&Settings::default()).context("init enigo")?;
    enigo.text(text).context("typing text")?;
    Ok(())
}
