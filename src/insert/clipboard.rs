use anyhow::{Result, anyhow};
use gtk::{gdk, prelude::*};

pub fn set_text(text: &str) -> Result<()> {
    let display = gdk::Display::default().ok_or_else(|| anyhow!("no display available"))?;
    let clipboard = display.clipboard();
    clipboard.set_text(text);
    Ok(())
}
