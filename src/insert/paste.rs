use std::{
    env,
    ffi::OsString,
    os::unix::fs::FileTypeExt,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, anyhow};

use crate::config::PasteMode;

pub fn is_available() -> bool {
    which::which("ydotool").is_ok()
}

pub fn trigger(mode: PasteMode) -> Result<()> {
    let binary = which::which("ydotool")
        .context("ydotool is not installed (auto-paste is optional and can be disabled)")?;
    let socket = select_socket(
        env::var_os("YDOTOOL_SOCKET"),
        env::var_os("XDG_RUNTIME_DIR"),
        Path::new("/tmp/.ydotool_socket"),
    );
    trigger_with_command(Command::new(binary), mode, socket.as_deref())
}

fn select_socket(
    explicit: Option<OsString>,
    runtime_dir: Option<OsString>,
    fallback: &Path,
) -> Option<PathBuf> {
    // Respect explicit user configuration, even if it is currently unavailable.
    if let Some(path) = explicit {
        return Some(path.into());
    }
    let runtime_socket = runtime_dir.map(|dir| PathBuf::from(dir).join(".ydotool_socket"));
    runtime_socket
        .filter(|path| is_socket(path))
        .or_else(|| is_socket(fallback).then(|| fallback.to_path_buf()))
}

fn is_socket(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|metadata| metadata.file_type().is_socket())
}

fn trigger_with_command(
    mut command: Command,
    mode: PasteMode,
    socket: Option<&Path>,
) -> Result<()> {
    command.arg("key").args(key_events(mode));
    if let Some(socket) = socket {
        command.env("YDOTOOL_SOCKET", socket);
    }
    let output = command.output().context("failed to execute ydotool")?;

    if output.status.success() {
        Ok(())
    } else {
        // Some distro builds print connection failures to stdout, not stderr.
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let diagnostics = [stderr.trim(), stdout.trim()]
            .into_iter()
            .filter(|message| !message.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        Err(anyhow!(
            "ydotool failed to paste using {} ({}): {}",
            mode.shortcut_label(),
            output.status,
            diagnostics
        ))
    }
}

fn key_events(mode: PasteMode) -> &'static [&'static str] {
    // Linux input keycodes: left Ctrl=29, left Shift=42, V=47.
    // Release in reverse order so neither modifier remains pressed.
    match mode {
        PasteMode::Standard => &["29:1", "47:1", "47:0", "29:0"],
        PasteMode::Terminal => &["29:1", "42:1", "47:1", "47:0", "42:0", "29:0"],
    }
}

#[cfg(test)]
mod tests {
    use std::{path::Path, process::Command};

    use super::{key_events, select_socket, trigger_with_command};
    use crate::config::PasteMode;

    #[test]
    fn standard_mode_keeps_ctrl_v() {
        assert_eq!(
            key_events(PasteMode::Standard),
            ["29:1", "47:1", "47:0", "29:0"]
        );
    }

    #[test]
    fn terminal_mode_releases_all_keys_in_reverse_order() {
        assert_eq!(
            key_events(PasteMode::Terminal),
            ["29:1", "42:1", "47:1", "47:0", "42:0", "29:0"]
        );
    }

    fn fake_ydotool(script: &str) -> Command {
        let mut command = Command::new("sh");
        command.args(["-c", script, "fake-ydotool"]);
        command
    }

    #[test]
    fn command_dispatches_the_selected_mode_without_injecting_real_keys() {
        for (mode, expected) in [
            (PasteMode::Standard, "key 29:1 47:1 47:0 29:0"),
            (PasteMode::Terminal, "key 29:1 42:1 47:1 47:0 42:0 29:0"),
        ] {
            let command = fake_ydotool(&format!("test \"$*\" = '{expected}'"));
            trigger_with_command(command, mode, None).unwrap();
        }
    }

    #[test]
    fn selected_socket_is_forwarded_to_the_client() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("ydotool.sock");
        let mut command = fake_ydotool("test \"$YDOTOOL_SOCKET\" = \"$EXPECTED_SOCKET\"");
        command.env("EXPECTED_SOCKET", &socket);
        trigger_with_command(command, PasteMode::Terminal, Some(&socket)).unwrap();
    }

    #[test]
    fn command_failure_reports_stdout_diagnostics() {
        let command = fake_ydotool("echo 'failed to connect socket'; exit 2");
        let error = trigger_with_command(command, PasteMode::Terminal, None)
            .unwrap_err()
            .to_string();
        assert!(error.contains("failed to connect socket"), "{error}");
    }

    #[test]
    fn socket_selection_prefers_runtime_and_falls_back_to_the_daemon_socket() {
        use std::os::unix::net::UnixListener;
        let dir = tempfile::tempdir().unwrap();
        let fallback = dir.path().join("system.sock");
        let _system = UnixListener::bind(&fallback).unwrap();
        let runtime_dir = Some(dir.path().as_os_str().to_owned());
        assert_eq!(
            select_socket(None, runtime_dir.clone(), &fallback),
            Some(fallback.clone())
        );
        let runtime = dir.path().join(".ydotool_socket");
        let _user = UnixListener::bind(&runtime).unwrap();
        assert_eq!(
            select_socket(None, runtime_dir.clone(), &fallback),
            Some(runtime)
        );
        let explicit = dir.path().join("custom.sock");
        assert_eq!(
            select_socket(
                Some(explicit.clone().into_os_string()),
                runtime_dir,
                &fallback
            ),
            Some(explicit)
        );
    }

    #[test]
    #[ignore = "requires a running local ydotoold; only requests client help, no key injection"]
    fn detected_socket_connects_without_sending_keys() {
        let socket = select_socket(
            std::env::var_os("YDOTOOL_SOCKET"),
            std::env::var_os("XDG_RUNTIME_DIR"),
            Path::new("/tmp/.ydotool_socket"),
        )
        .expect("a configured daemon socket must exist");
        let output = std::process::Command::new(which::which("ydotool").unwrap())
            .env("YDOTOOL_SOCKET", socket)
            .args(["key", "--help"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn regular_files_are_not_selected_as_sockets() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".ydotool_socket");
        std::fs::write(&path, "not a socket").unwrap();
        assert_eq!(
            select_socket(None, Some(dir.path().as_os_str().to_owned()), &path),
            None
        );
    }

    #[test]
    fn command_failure_includes_shortcut_and_daemon_error() {
        let command = fake_ydotool("echo 'socket permission denied' >&2; exit 1");
        let error = trigger_with_command(command, PasteMode::Terminal, None)
            .unwrap_err()
            .to_string();
        assert!(error.contains("Ctrl+Shift+V"));
        assert!(error.contains("socket permission denied"));
    }
}
