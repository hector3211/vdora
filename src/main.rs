#[cfg(target_os = "linux")]
mod app;
mod audio;
mod config;
#[cfg(target_os = "linux")]
mod diagnostics;
#[cfg(target_os = "linux")]
mod hotkey;
#[cfg(target_os = "linux")]
mod insert;
#[cfg(target_os = "linux")]
mod oneshot;
#[cfg(target_os = "linux")]
mod state;
mod stt;
#[cfg(target_os = "linux")]
mod tray;
#[cfg(windows)]
mod windows;

#[cfg(not(any(target_os = "linux", windows)))]
compile_error!("Vdora supports Linux and Windows only");

use anyhow::Result;
use config::AppConfig;
#[cfg(target_os = "linux")]
use gtk::prelude::*;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

#[cfg(target_os = "linux")]
fn main() -> Result<()> {
    match oneshot::parse_args() {
        Ok(oneshot::Mode::Gui) => {
            init_tracing(&AppConfig::load_or_default());
            let app = adw::Application::builder()
                .application_id("com.vdora.App")
                .build();

            app.connect_activate(app::build_ui);
            app.run();
        }
        Ok(oneshot::Mode::Run(options)) => {
            let config = AppConfig::load_or_default();
            init_tracing(&config);
            oneshot::run(options, config)?;
        }
        Ok(oneshot::Mode::Help) => {
            oneshot::print_help();
        }
        Err(err) => {
            eprintln!("Error: {err}");
            std::process::exit(2);
        }
    }

    Ok(())
}

#[cfg(windows)]
fn main() -> Result<()> {
    windows::run()
}

fn init_tracing(config: &AppConfig) {
    let fallback_level = config.log_level.as_filter_directive();
    let env_filter = match std::env::var("RUST_LOG") {
        Ok(filter) => EnvFilter::new(filter),
        Err(_) => EnvFilter::new(fallback_level),
    };

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(env_filter)
        .init();
}
