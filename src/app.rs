use std::{
    cell::RefCell,
    fs,
    path::PathBuf,
    rc::Rc,
    sync::mpsc,
    thread,
    time::Duration,
};

use adw::prelude::*;
use anyhow::{Context, Result, anyhow};
use gtk::{Orientation, gdk, gio, glib};

use crate::{
    audio::recorder::{Recorder, RecordingSession},
    config::AppConfig,
    hotkey,
    insert::{clipboard, paste},
    state::AppState,
    stt::whisper::WhisperService,
    tray::TrayEvent,
};

enum AppMessage {
    TranscriptionFinished(Result<String, String>),
}

pub fn build_ui(app: &adw::Application) {
    let config = Rc::new(RefCell::new(AppConfig::load_or_default()));
    let state = Rc::new(RefCell::new(AppState::Idle));
    let recorder = Rc::new(Recorder::new());
    let active_session: Rc<RefCell<Option<RecordingSession>>> = Rc::new(RefCell::new(None));

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Vdora")
        .default_width(620)
        .default_height(520)
        .build();

    let overlay = adw::ToastOverlay::new();
    window.set_content(Some(&overlay));

    let root = gtk::Box::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .margin_top(16)
        .margin_bottom(16)
        .margin_start(16)
        .margin_end(16)
        .build();

    let stack = adw::ViewStack::new();
    stack.set_vexpand(true);
    let switcher = adw::ViewSwitcher::new();
    switcher.set_stack(Some(&stack));
    switcher.set_policy(adw::ViewSwitcherPolicy::Wide);
    root.append(&switcher);
    root.append(&stack);
    overlay.set_child(Some(&root));

    let recorder_page = gtk::Box::builder()
        .orientation(Orientation::Vertical)
        .spacing(16)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();

    let title = gtk::Label::builder()
        .label("Voice to Cursor")
        .css_classes(["title-1"])
        .xalign(0.0)
        .build();
    recorder_page.append(&title);

    let subtitle = gtk::Label::builder()
        .label("Record, transcribe locally, then paste where your cursor is.")
        .css_classes(["dim-label"])
        .xalign(0.0)
        .wrap(true)
        .build();
    recorder_page.append(&subtitle);

    let status = gtk::Label::builder()
        .label(AppState::Idle.label())
        .xalign(0.0)
        .build();
    recorder_page.append(&status);

    let record_button = gtk::Button::builder()
        .label("Start Recording")
        .hexpand(true)
        .build();
    record_button.add_css_class("suggested-action");
    recorder_page.append(&record_button);

    let options_row = gtk::Box::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .build();
    let autopaste_switch = gtk::Switch::builder()
        .active(config.borrow().autopaste)
        .valign(gtk::Align::Center)
        .build();
    let autopaste_label = gtk::Label::builder()
        .label("Auto-paste after transcription")
        .xalign(0.0)
        .hexpand(true)
        .build();
    options_row.append(&autopaste_label);
    options_row.append(&autopaste_switch);
    recorder_page.append(&options_row);

    let model_label = gtk::Label::builder()
        .label(format!("Model: {}", config.borrow().model_path.display()))
        .css_classes(["dim-label"])
        .xalign(0.0)
        .wrap(true)
        .build();
    recorder_page.append(&model_label);

    let hotkey_label = gtk::Label::builder()
        .label(format_hotkey_line(&config.borrow()))
        .css_classes(["dim-label"])
        .xalign(0.0)
        .wrap(true)
        .build();
    recorder_page.append(&hotkey_label);

    let transcript_title = gtk::Label::builder()
        .label("Last transcript")
        .xalign(0.0)
        .build();
    recorder_page.append(&transcript_title);

    let transcript_view = gtk::TextView::builder()
        .editable(false)
        .wrap_mode(gtk::WrapMode::WordChar)
        .vexpand(true)
        .build();
    let transcript_buffer = gtk::TextBuffer::new(None);
    transcript_view.set_buffer(Some(&transcript_buffer));
    let transcript_scroll = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hexpand(true)
        .min_content_height(180)
        .child(&transcript_view)
        .build();
    recorder_page.append(&transcript_scroll);

    stack.add_titled(&recorder_page, Some("recorder"), "Recorder");

    let settings_page = gtk::Box::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();

    let settings_title = gtk::Label::builder()
        .label("Settings")
        .css_classes(["title-2"])
        .xalign(0.0)
        .build();
    settings_page.append(&settings_title);

    let settings_subtitle = gtk::Label::builder()
        .label("Tune transcription and hotkey behavior.")
        .css_classes(["dim-label"])
        .xalign(0.0)
        .build();
    settings_page.append(&settings_subtitle);

    let settings_autopaste_row = gtk::Box::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .build();
    let settings_autopaste_label = gtk::Label::builder()
        .label("Auto-paste")
        .xalign(0.0)
        .hexpand(true)
        .build();
    let settings_autopaste_switch = gtk::Switch::builder()
        .active(config.borrow().autopaste)
        .valign(gtk::Align::Center)
        .build();
    settings_autopaste_row.append(&settings_autopaste_label);
    settings_autopaste_row.append(&settings_autopaste_switch);
    settings_page.append(&settings_autopaste_row);

    let model_entry = gtk::Entry::builder()
        .hexpand(true)
        .text(config.borrow().model_path.display().to_string())
        .placeholder_text("/home/user/.local/share/vdora/models/ggml-base.en.bin")
        .build();
    settings_page.append(&labeled_row("Model path", &model_entry));

    let language_entry = gtk::Entry::builder()
        .hexpand(true)
        .text(config.borrow().language.clone().unwrap_or_default())
        .placeholder_text("auto (blank), en, es, ...")
        .build();
    settings_page.append(&labeled_row("Language", &language_entry));

    let hotkey_entry = gtk::Entry::builder()
        .hexpand(true)
        .text(config.borrow().hotkey.clone())
        .placeholder_text("Ctrl+Alt+Space or <Ctrl><Alt>space")
        .build();
    settings_page.append(&labeled_row("Record toggle hotkey", &hotkey_entry));

    let save_button = gtk::Button::with_label("Save Settings");
    save_button.add_css_class("suggested-action");
    settings_page.append(&save_button);

    let settings_hint = gtk::Label::builder()
        .label(settings_hint_text())
        .css_classes(["dim-label"])
        .xalign(0.0)
        .wrap(true)
        .build();
    settings_page.append(&settings_hint);

    stack.add_titled(&settings_page, Some("settings"), "Settings");

    let (sender, receiver) = mpsc::channel::<AppMessage>();
    let (tray_sender, tray_receiver) = mpsc::channel::<TrayEvent>();
    let tray_controller = match crate::tray::spawn(tray_sender) {
        Ok(controller) => Some(controller),
        Err(err) => {
            tracing::warn!("failed to start tray service: {err}");
            None
        }
    };

    {
        let status = status.clone();
        let state = state.clone();
        let overlay = overlay.clone();
        let transcript_buffer = transcript_buffer.clone();
        let autopaste_switch = autopaste_switch.clone();
        let record_button = record_button.clone();
        let window = window.clone();
        let app = app.clone();
        let tray_controller = tray_controller.clone();

        glib::timeout_add_local(Duration::from_millis(50), move || {
            while let Ok(event) = tray_receiver.try_recv() {
                match event {
                    TrayEvent::ShowWindow => {
                        window.present();
                    }
                    TrayEvent::HideWindow => {
                        window.hide();
                    }
                    TrayEvent::ToggleRecording => {
                        app.activate_action("toggle-recording", None::<&glib::Variant>);
                    }
                    TrayEvent::Quit => {
                        app.quit();
                    }
                }
            }

            while let Ok(message) = receiver.try_recv() {
                match message {
                    AppMessage::TranscriptionFinished(result) => {
                        *state.borrow_mut() = AppState::Idle;
                        status.set_label(AppState::Idle.label());
                        record_button.set_label("Start Recording");
                        if let Some(controller) = &tray_controller {
                            controller.set_status(AppState::Idle.label());
                        }

                        match result {
                            Ok(transcript) => {
                                if transcript.trim().is_empty() {
                                    transcript_buffer.set_text("");
                                    add_toast(&overlay, "No speech detected. Ready when you are.");
                                    continue;
                                }

                                transcript_buffer.set_text(&transcript);

                                if let Err(err) = clipboard::set_text(&transcript) {
                                    add_toast(&overlay, &format!("Clipboard error: {err}"));
                                } else if autopaste_switch.is_active() {
                                    match paste::trigger_ctrl_v() {
                                        Ok(()) => add_toast(&overlay, "Transcribed and pasted"),
                                        Err(err) => add_toast(
                                            &overlay,
                                            &format!(
                                                "Copied only. Auto-paste unavailable: {err}"
                                            ),
                                        ),
                                    }
                                } else {
                                    add_toast(&overlay, "Transcribed and copied")
                                }
                            }
                            Err(err) => {
                                transcript_buffer.set_text("");
                                *state.borrow_mut() = AppState::Error;
                                status.set_label(AppState::Error.label());
                                if let Some(controller) = &tray_controller {
                                    controller.set_status(AppState::Error.label());
                                }
                                add_toast(&overlay, &err);
                            }
                        }
                    }
                }
            }
            glib::ControlFlow::Continue
        });
    }

    {
        let config = config.clone();
        let overlay = overlay.clone();
        let settings_autopaste_switch = settings_autopaste_switch.clone();
        autopaste_switch.connect_active_notify(move |switch| {
            settings_autopaste_switch.set_active(switch.is_active());
            config.borrow_mut().autopaste = switch.is_active();
            if let Err(err) = config.borrow().save() {
                add_toast(&overlay, &format!("Failed to save settings: {err}"));
            }
        });
    }

    {
        let config = config.clone();
        let overlay = overlay.clone();
        let autopaste_switch = autopaste_switch.clone();
        settings_autopaste_switch.connect_active_notify(move |switch| {
            autopaste_switch.set_active(switch.is_active());
            config.borrow_mut().autopaste = switch.is_active();
            if let Err(err) = config.borrow().save() {
                add_toast(&overlay, &format!("Failed to save settings: {err}"));
            }
        });
    }

    let toggle_recording: Rc<dyn Fn()> = {
        let status = status.clone();
        let overlay = overlay.clone();
        let sender = sender.clone();
        let config = config.clone();
        let recorder = recorder.clone();
        let active_session = active_session.clone();
        let state = state.clone();
        let record_button = record_button.clone();
        let tray_controller = tray_controller.clone();

        Rc::new(move || {
            let currently_recording = active_session.borrow().is_some();

            if !currently_recording {
                match recorder.start() {
                    Ok(session) => {
                        *active_session.borrow_mut() = Some(session);
                        *state.borrow_mut() = AppState::Recording;
                        status.set_label(AppState::Recording.label());
                        if let Some(controller) = &tray_controller {
                            controller.set_status(AppState::Recording.label());
                        }
                        record_button.set_label("Stop Recording");
                        add_toast(&overlay, "Listening...");
                    }
                    Err(err) => {
                        *state.borrow_mut() = AppState::Error;
                        status.set_label(AppState::Error.label());
                        if let Some(controller) = &tray_controller {
                            controller.set_status(AppState::Error.label());
                        }
                        add_toast(&overlay, &format!("Unable to record: {err}"));
                    }
                }
                return;
            }

            let Some(session) = active_session.borrow_mut().take() else {
                add_toast(&overlay, "No active recording session");
                return;
            };

            record_button.set_label("Start Recording");
            *state.borrow_mut() = AppState::Transcribing;
            status.set_label(AppState::Transcribing.label());
            if let Some(controller) = &tray_controller {
                controller.set_status(AppState::Transcribing.label());
            }

            let sender = sender.clone();
            let config_snapshot = config.borrow().clone();
            thread::spawn(move || {
                let result = transcribe_session(session, config_snapshot)
                    .map_err(|err| format!("Transcription failed: {err:#}"));

                if sender
                    .send(AppMessage::TranscriptionFinished(result))
                    .is_err()
                {
                    tracing::error!("failed to send transcription result to UI thread");
                }
            });
        })
    };

    {
        let toggle_recording = toggle_recording.clone();
        record_button.connect_clicked(move |_| {
            toggle_recording();
        });
    }

    let toggle_action = gio::SimpleAction::new("toggle-recording", None);
    {
        let toggle_recording = toggle_recording.clone();
        toggle_action.connect_activate(move |_, _| {
            toggle_recording();
        });
    }
    app.add_action(&toggle_action);

    let initial_hotkey = config.borrow().hotkey.clone();
    if let Err(err) = apply_hotkey_accel(app, &initial_hotkey) {
        add_toast(&overlay, &format!("Invalid hotkey in config: {err}. Reset to default."));
        let default = hotkey::default_hotkey().to_string();
        if let Err(reset_err) = apply_hotkey_accel(app, &default) {
            add_toast(
                &overlay,
                &format!("Failed to apply default hotkey: {reset_err}"),
            );
        } else {
            config.borrow_mut().hotkey = default;
            let _ = config.borrow().save();
            hotkey_label.set_label(&format_hotkey_line(&config.borrow()));
            hotkey_entry.set_text(&config.borrow().hotkey);
        }
    }

    {
        let app = app.clone();
        let config = config.clone();
        let overlay = overlay.clone();
        let model_label = model_label.clone();
        let hotkey_label = hotkey_label.clone();
        let autopaste_switch = autopaste_switch.clone();
        let settings_autopaste_switch = settings_autopaste_switch.clone();
        let model_entry = model_entry.clone();
        let language_entry = language_entry.clone();
        let hotkey_entry = hotkey_entry.clone();

        save_button.connect_clicked(move |_| {
            let model_text = model_entry.text().trim().to_string();
            if model_text.is_empty() {
                add_toast(&overlay, "Model path cannot be empty");
                return;
            }
            let model_path = PathBuf::from(&model_text);
            if let Err(err) = validate_model_path(&model_path) {
                add_toast(&overlay, &format!("Invalid model path: {err}"));
                return;
            }

            let language_text = language_entry.text().trim().to_string();
            let hotkey_text = hotkey_entry.text().trim().to_string();
            let autopaste_enabled = settings_autopaste_switch.is_active();

            let normalized_hotkey = match parse_hotkey_accel(&hotkey_text) {
                Ok(v) => v,
                Err(err) => {
                    add_toast(&overlay, &format!("Invalid hotkey: {err}"));
                    return;
                }
            };

            let mut next_cfg = config.borrow().clone();
            next_cfg.model_path = model_path;
            next_cfg.language = if language_text.is_empty() {
                None
            } else {
                Some(language_text)
            };
            next_cfg.autopaste = autopaste_enabled;
            next_cfg.hotkey = normalized_hotkey;

            if let Err(err) = next_cfg.save() {
                add_toast(&overlay, &format!("Failed to save settings: {err}"));
                return;
            }

            app.set_accels_for_action("app.toggle-recording", &[&next_cfg.hotkey]);

            {
                let mut cfg = config.borrow_mut();
                *cfg = next_cfg;
            }

            model_label.set_label(&format!("Model: {}", config.borrow().model_path.display()));
            hotkey_label.set_label(&format_hotkey_line(&config.borrow()));
            autopaste_switch.set_active(config.borrow().autopaste);
            settings_autopaste_switch.set_active(config.borrow().autopaste);

            add_toast(&overlay, "Settings saved");
        });
    }

    window.connect_close_request(|window| {
        window.hide();
        glib::Propagation::Stop
    });

    window.present();
}

fn apply_hotkey_accel(app: &adw::Application, input: &str) -> Result<String> {
    let accel = parse_hotkey_accel(input)?;
    app.set_accels_for_action("app.toggle-recording", &[&accel]);
    Ok(accel)
}

fn parse_hotkey_accel(input: &str) -> Result<String> {
    let accel = hotkey::to_gtk_accelerator(input).map_err(|msg| anyhow!(msg))?;
    let Some((key, mods)) = gtk::accelerator_parse(&accel) else {
        return Err(anyhow!("could not parse accelerator string"));
    };
    if key == gdk::Key::VoidSymbol || mods.is_empty() {
        return Err(anyhow!("use at least one modifier and one key"));
    }
    Ok(accel)
}

fn labeled_row(label_text: &str, child: &impl IsA<gtk::Widget>) -> gtk::Box {
    let row = gtk::Box::builder()
        .orientation(Orientation::Vertical)
        .spacing(6)
        .build();
    let label = gtk::Label::builder().label(label_text).xalign(0.0).build();
    row.append(&label);
    row.append(child);
    row
}

fn format_hotkey_line(config: &AppConfig) -> String {
    format!(
        "Hotkey: {} ({})",
        config.hotkey,
        if hotkey::global_hotkeys_supported() {
            "X11 session: app-level shortcut"
        } else {
            "Wayland session: app-level shortcut"
        }
    )
}

fn settings_hint_text() -> &'static str {
    "Hotkey accepts forms like Ctrl+Alt+Space or <Ctrl><Alt>space. On Wayland this works while the app is focused; use GNOME custom shortcuts for global behavior."
}

fn add_toast(overlay: &adw::ToastOverlay, text: &str) {
    let toast = adw::Toast::new(text);
    overlay.add_toast(toast);
}

fn transcribe_session(session: RecordingSession, config: AppConfig) -> Result<String> {
    let wav_path = session.stop().context("failed to finish audio recording")?;
    let result = transcribe_file(wav_path.clone(), &config);
    cleanup_recording_file(&wav_path);
    result
}

fn cleanup_recording_file(path: &PathBuf) {
    if let Err(err) = fs::remove_file(path) {
        tracing::warn!("failed to clean up recording file {}: {err}", path.display());
    }
}

fn validate_model_path(path: &PathBuf) -> Result<()> {
    let metadata = fs::metadata(path)
        .with_context(|| format!("failed to read model metadata at {}", path.display()))?;
    if !metadata.is_file() {
        return Err(anyhow!("model path must point to a regular file"));
    }
    Ok(())
}

fn transcribe_file(wav_path: PathBuf, config: &AppConfig) -> Result<String> {
    let whisper = WhisperService::new(config.model_path.clone(), config.language.clone());
    whisper.transcribe_file(&wav_path)
}
