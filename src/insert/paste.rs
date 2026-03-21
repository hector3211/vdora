use std::process::Command;

use anyhow::{Context, Result, anyhow};

pub fn is_available() -> bool {
    which::which("ydotool").is_ok()
}

pub fn trigger_ctrl_v() -> Result<()> {
    let binary = which::which("ydotool")
        .context("ydotool is not installed (auto-paste is optional and can be disabled)")?;
    let output = Command::new(binary)
        .args(["key", "29:1", "47:1", "47:0", "29:0"])
        .output()
        .context("failed to execute ydotool")?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(anyhow!("ydotool failed: {stderr}"))
    }
}
