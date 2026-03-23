use std::{
    cell::RefCell,
    collections::VecDeque,
    env, fs,
    io::Write,
    path::PathBuf,
    rc::Rc,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use directories::ProjectDirs;

use crate::config::AppConfig;

const APP_QUALIFIER: &str = "com";
const APP_ORG: &str = "vdora";
const APP_NAME: &str = "vdora";

#[derive(Debug, Clone)]
pub struct ReportEntry {
    pub unix_seconds: u64,
    pub source: String,
    pub message: String,
}

#[derive(Clone)]
pub struct Reporter {
    entries: Rc<RefCell<VecDeque<ReportEntry>>>,
    max_entries: usize,
}

impl Reporter {
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: Rc::new(RefCell::new(VecDeque::new())),
            max_entries,
        }
    }

    pub fn record_error(&self, source: &str, message: &str) {
        let mut entries = self.entries.borrow_mut();
        entries.push_back(ReportEntry {
            unix_seconds: now_unix_seconds(),
            source: source.to_string(),
            message: message.to_string(),
        });
        while entries.len() > self.max_entries {
            entries.pop_front();
        }
    }

    pub fn snapshot(&self) -> Vec<ReportEntry> {
        self.entries.borrow().iter().cloned().collect()
    }

}

#[derive(Debug, Clone)]
pub struct HealthSnapshot {
    pub session_type: String,
    pub pw_record_available: bool,
    pub wl_copy_available: Option<bool>,
    pub ydotool_available: bool,
    pub model_ready: bool,
}

impl HealthSnapshot {
    pub fn failure_count(&self) -> usize {
        let mut failures = 0usize;
        if !self.pw_record_available {
            failures += 1;
        }
        if !self.model_ready {
            failures += 1;
        }
        failures
    }
}

pub fn collect_health(model_ready: bool) -> HealthSnapshot {
    let session_type = env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "unknown".to_string());
    let is_wayland = session_type.eq_ignore_ascii_case("wayland") || env::var_os("WAYLAND_DISPLAY").is_some();

    HealthSnapshot {
        session_type,
        pw_record_available: which::which("pw-record").is_ok(),
        wl_copy_available: if is_wayland {
            Some(which::which("wl-copy").is_ok())
        } else {
            None
        },
        ydotool_available: which::which("ydotool").is_ok(),
        model_ready,
    }
}

pub fn health_lines(health: &HealthSnapshot) -> Vec<String> {
    let mut lines = Vec::new();
    lines.push(format!(
        "Session type: {}",
        if health.session_type.trim().is_empty() {
            "unknown"
        } else {
            &health.session_type
        }
    ));
    lines.push(format!(
        "pw-record: {}",
        bool_status(health.pw_record_available)
    ));
    if let Some(wl_copy_available) = health.wl_copy_available {
        lines.push(format!("wl-copy: {}", bool_status(wl_copy_available)));
    } else {
        lines.push("wl-copy: n/a (non-Wayland session)".to_string());
    }
    lines.push(format!("ydotool: {}", bool_status(health.ydotool_available)));
    lines.push(format!("model file: {}", bool_status(health.model_ready)));
    lines
}

pub fn diagnostics_bundle(config: &AppConfig, health: &HealthSnapshot, errors: &[ReportEntry]) -> String {
    let mut lines = vec![
        "Vdora diagnostics bundle".to_string(),
        format!("generated_at_unix: {}", now_unix_seconds()),
        format!("app_version: {}", env!("CARGO_PKG_VERSION")),
        format!("session_type: {}", health.session_type),
        format!("model_path: {}", config.model_path.display()),
        format!("language: {}", config.language.as_deref().unwrap_or("auto")),
        format!("autopaste_enabled: {}", config.autopaste),
        format!("hotkey: {}", config.hotkey),
        format!("log_level: {}", config.log_level.as_ui_label()),
        format!("pw_record_available: {}", health.pw_record_available),
        format!("wl_copy_available: {}", health.wl_copy_available.map(|v| v.to_string()).unwrap_or_else(|| "n/a".to_string())),
        format!("ydotool_available: {}", health.ydotool_available),
        format!("model_ready: {}", health.model_ready),
        "errors:".to_string(),
    ];

    if errors.is_empty() {
        lines.push("  - none".to_string());
    } else {
        for entry in errors {
            lines.push(format!(
                "  - [{}] [{}] {}",
                entry.unix_seconds, entry.source, entry.message
            ));
        }
    }

    lines.join("\n")
}

pub fn export_diagnostics_bundle(bundle: &str) -> Result<PathBuf> {
    let state_dir = diagnostics_state_dir()?;
    ensure_private_state_dir(&state_dir)?;

    let now_millis = now_unix_millis();
    for nonce in 0..32u32 {
        let path = state_dir.join(diagnostics_file_name(now_millis, nonce));
        match write_bundle_create_new(&path, bundle) {
            Ok(()) => return Ok(path),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("failed to write diagnostics bundle at {}", path.display()));
            }
        }
    }

    Err(anyhow::anyhow!(
        "failed to create unique diagnostics file after multiple attempts"
    ))
}

fn diagnostics_state_dir() -> Result<PathBuf> {
    let project_dirs = ProjectDirs::from(APP_QUALIFIER, APP_ORG, APP_NAME)
        .context("could not determine user state directory")?;
    Ok(project_dirs
        .state_dir()
        .map(|path| path.to_path_buf())
        .unwrap_or_else(|| project_dirs.data_local_dir().to_path_buf()))
}

fn diagnostics_file_name(now_millis: u128, nonce: u32) -> String {
    if nonce == 0 {
        format!("diagnostics-{now_millis}.log")
    } else {
        format!("diagnostics-{now_millis}-{nonce}.log")
    }
}

fn write_bundle_create_new(path: &PathBuf, bundle: &str) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(bundle.as_bytes())?;
        file.flush()?;
        return Ok(());
    }

    #[cfg(not(unix))]
    {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)?;
        file.write_all(bundle.as_bytes())?;
        file.flush()?;
        Ok(())
    }
}

fn ensure_private_state_dir(state_dir: &PathBuf) -> Result<()> {
    fs::create_dir_all(state_dir)
        .with_context(|| format!("failed to create diagnostics directory at {}", state_dir.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(state_dir, fs::Permissions::from_mode(0o700)).with_context(|| {
            format!(
                "failed to set permissions on diagnostics directory {}",
                state_dir.display()
            )
        })?;
    }

    Ok(())
}

fn bool_status(available: bool) -> &'static str {
    if available {
        "available"
    } else {
        "missing"
    }
}

fn now_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn now_unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{
        diagnostics_bundle, diagnostics_file_name, health_lines, HealthSnapshot, ReportEntry,
        Reporter,
    };
    use crate::config::{AppConfig, LogLevel};

    #[test]
    fn reporter_keeps_only_latest_entries() {
        let reporter = Reporter::new(2);
        reporter.record_error("test", "one");
        reporter.record_error("test", "two");
        reporter.record_error("test", "three");

        let snapshot = reporter.snapshot();
        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot[0].message, "two");
        assert_eq!(snapshot[1].message, "three");
    }

    #[test]
    fn health_lines_include_expected_sections() {
        let health = HealthSnapshot {
            session_type: "wayland".to_string(),
            pw_record_available: true,
            wl_copy_available: Some(false),
            ydotool_available: true,
            model_ready: false,
        };

        let lines = health_lines(&health).join("\n");
        assert!(lines.contains("Session type: wayland"));
        assert!(lines.contains("pw-record: available"));
        assert!(lines.contains("wl-copy: missing"));
        assert!(lines.contains("model file: missing"));
    }

    #[test]
    fn bundle_contains_config_and_errors() {
        let mut config = AppConfig::default();
        config.log_level = LogLevel::Debug;
        let health = HealthSnapshot {
            session_type: "x11".to_string(),
            pw_record_available: true,
            wl_copy_available: None,
            ydotool_available: false,
            model_ready: true,
        };
        let errors = vec![ReportEntry {
            unix_seconds: 123,
            source: "recorder".to_string(),
            message: "failed to start".to_string(),
        }];

        let bundle = diagnostics_bundle(&config, &health, &errors);
        assert!(bundle.contains("log_level: debug"));
        assert!(bundle.contains("errors:"));
        assert!(bundle.contains("failed to start"));
    }

    #[test]
    fn diagnostics_file_names_are_unique_for_nonce() {
        let a = diagnostics_file_name(1234, 0);
        let b = diagnostics_file_name(1234, 1);
        assert_ne!(a, b);
        assert!(a.ends_with(".log"));
        assert!(b.ends_with(".log"));
    }
}
