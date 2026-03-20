mod app;
mod audio;
mod config;
mod hotkey;
mod insert;
mod state;
mod stt;
mod tray;

use anyhow::Result;
use gtk::prelude::*;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

fn main() -> Result<()> {
    init_tracing();

    let app = adw::Application::builder()
        .application_id("com.vdora.App")
        .build();

    app.connect_activate(app::build_ui);
    app.run();

    Ok(())
}

fn init_tracing() {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(EnvFilter::from_default_env())
        .init();
}
