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

    // Without --foreground the launcher exits after setting the selection.
    // A running launcher isn't a successful copy; auto-paste must wait for it.
    run_clipboard_helper(
        Command::new("wl-copy").args(["--type", "text/plain;charset=utf-8"]),
        "wl-copy",
        text,
    )
}

fn set_with_xsel(text: &str) -> Result<()> {
    run_clipboard_helper(
        Command::new("xsel").args(["--clipboard", "--input"]),
        "xsel",
        text,
    )
}

fn run_clipboard_helper(command: &mut Command, name: &str, text: &str) -> Result<()> {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("failed to spawn {name}"))?;

    let write_result = child
        .stdin
        .take()
        .with_context(|| format!("failed to open stdin for {name}"))
        .and_then(|mut stdin| {
            stdin
                .write_all(text.as_bytes())
                .with_context(|| format!("failed to write clipboard text to {name}"))
        });
    // Reap the launcher even when writing fails.
    let status = child
        .wait()
        .with_context(|| format!("failed to wait for {name}"))?;
    if !status.success() {
        return Err(anyhow!("{name} exited with status {status}"));
    }
    write_result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_for_clipboard_helper_and_reports_delayed_failure() {
        let mut command = Command::new("sh");
        command.args(["-c", "cat > /dev/null; sleep 0.05; exit 7"]);
        let error = run_clipboard_helper(&mut command, "test helper", "transcript").unwrap_err();
        assert!(error.to_string().contains("exit status: 7"));
    }

    #[test]
    fn sends_the_whole_transcript_before_waiting() {
        let mut command = Command::new("sh");
        command.args(["-c", "test \"$(wc -c)\" -eq 131072"]);
        run_clipboard_helper(&mut command, "test helper", &"a".repeat(131_072)).unwrap();
    }
}
