use std::{
    io::Write,
    process::{Command, Stdio},
};

use anyhow::{Context, Result, anyhow};
use gtk::gdk;
use gtk::prelude::DisplayExt;

pub fn set_text(text: &str) -> Result<()> {
    if let Ok(()) = set_with_wl_copy(text) {
        return Ok(());
    }

    if let Ok(()) = set_with_xsel(text) {
        return Ok(());
    }

    set_with_gdk(text)
}

fn set_with_gdk(text: &str) -> Result<()> {
    let display = gdk::Display::default().ok_or_else(|| anyhow!("no display available"))?;
    let clipboard = display.clipboard();
    clipboard.set_text(text);
    Ok(())
}

fn set_with_wl_copy(text: &str) -> Result<()> {
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return Err(anyhow!("not running on Wayland"));
    }

    let wl_copy = which::which("wl-copy").context("wl-copy is not installed")?;
    let mut child = Command::new(wl_copy)
        .arg("--type")
        .arg("text/plain;charset=utf-8")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to spawn wl-copy")?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("failed to open stdin for wl-copy"))?;
    stdin
        .write_all(text.as_bytes())
        .context("failed to write clipboard text to wl-copy")?;
    drop(stdin);

    if let Some(status) = child.try_wait().context("failed to check wl-copy status")? {
        if !status.success() {
            return Err(anyhow!("wl-copy exited with status {status}"));
        }
    }

    Ok(())
}

fn set_with_xsel(text: &str) -> Result<()> {
    let xsel = which::which("xsel").context("xsel is not installed")?;
    let mut child = Command::new(xsel)
        .arg("--clipboard")
        .arg("--input")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to spawn xsel")?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("failed to open stdin for xsel"))?;
    stdin
        .write_all(text.as_bytes())
        .context("failed to write clipboard text to xsel")?;
    drop(stdin);

    let status = child.wait().context("failed to wait for xsel")?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("xsel exited with status {status}"))
    }
}
