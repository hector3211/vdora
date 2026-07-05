mod app;
mod audio;
mod config;
mod diagnostics;
mod hotkey;
mod insert;
mod oneshot;
mod state;
mod stt;
mod tray;

use anyhow::Result;
use gtk::prelude::*;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

fn main() -> Result<()> {
    init_tracing();

    match oneshot::parse_args() {
        Ok(oneshot::Mode::Gui) => {
            let app = adw::Application::builder()
                .application_id("com.vdora.App")
                .build();

            app.connect_activate(app::build_ui);
            app.run();
        }
        Ok(oneshot::Mode::Run(options)) => {
            oneshot::run(options)?;
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

fn init_tracing() {
    let config = crate::config::AppConfig::load_or_default();
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
