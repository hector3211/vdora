use std::{
    cell::RefCell,
    fs,
    path::PathBuf,
    rc::Rc,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use adw::prelude::*;
use anyhow::{anyhow, Context, Result};
use gtk::{gdk, gio, glib, Orientation};

use crate::{
    audio::recorder::{cleanup_stale_recordings, Recorder, RecordingSession},
    config::{AppConfig, LogLevel},
    diagnostics,
    hotkey,
    insert::{clipboard, paste},
    state::AppState,
    stt::whisper::WhisperService,
    tray::TrayEvent,
};

const STALE_RECORDING_MAX_AGE: Duration = Duration::from_secs(10 * 60);

enum AppMessage {
    TranscriptionFinished { id: u64, result: Result<String, String> },
}

pub fn build_ui(app: &adw::Application) {
    let removed = cleanup_stale_recordings(STALE_RECORDING_MAX_AGE);
    if removed > 0 {
        tracing::info!("removed {removed} stale recording file(s) from temp directory");
    }

    let config = Rc::new(RefCell::new(AppConfig::load_or_default()));
    let reporter = diagnostics::Reporter::new(25);
    let autopaste_available = paste::is_available();
    if config.borrow().autopaste && !autopaste_available {
        tracing::info!("ydotool not found, disabling persisted auto-paste setting");
        config.borrow_mut().autopaste = false;
        if let Err(err) = config.borrow().save() {
            tracing::warn!("failed to persist disabled auto-paste setting: {err}");
        }
    }
    let state = Rc::new(RefCell::new(AppState::Idle));
    let recorder = Rc::new(Recorder::new());
    let active_session: Rc<RefCell<Option<RecordingSession>>> = Rc::new(RefCell::new(None));
    let recording_started_at: Rc<RefCell<Option<Instant>>> = Rc::new(RefCell::new(None));
    let next_transcription_id: Rc<RefCell<u64>> = Rc::new(RefCell::new(1));
    let current_transcription_id: Rc<RefCell<Option<u64>>> = Rc::new(RefCell::new(None));

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

    let status_row = gtk::Box::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .build();
    let status_spinner = gtk::Spinner::new();
    status_spinner.set_spinning(false);
    let status = gtk::Label::builder()
        .label(AppState::Idle.label())
        .xalign(0.0)
        .build();
    status_row.append(&status_spinner);
    status_row.append(&status);
    recorder_page.append(&status_row);

    let elapsed_label = gtk::Label::builder()
        .label("")
        .css_classes(["dim-label"])
        .xalign(0.0)
        .build();
    recorder_page.append(&elapsed_label);

    let record_button = gtk::Button::builder()
        .label("Start Recording")
        .hexpand(true)
        .build();
    record_button.add_css_class("suggested-action");
    recorder_page.append(&record_button);

    let recorder_hint = gtk::Label::builder()
        .label(format_hotkey_tip(&config.borrow()))
        .css_classes(["dim-label"])
        .xalign(0.0)
        .wrap(true)
        .build();
    recorder_page.append(&recorder_hint);

    let model_status_label = gtk::Label::builder()
        .label(format_model_status(config.borrow().model_path.is_file()))
        .css_classes(["dim-label"])
        .xalign(0.0)
        .wrap(true)
        .build();
    recorder_page.append(&model_status_label);

    let autopaste_status_label = gtk::Label::builder()
        .label(format_autopaste_status(autopaste_available))
        .css_classes(["dim-label"])
        .xalign(0.0)
        .wrap(true)
        .build();
    recorder_page.append(&autopaste_status_label);

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

    let transcript_hint = gtk::Label::builder()
        .label("Your latest transcript appears here.")
        .css_classes(["dim-label"])
        .xalign(0.0)
        .build();
    recorder_page.append(&transcript_hint);

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

    let transcript_actions_row = gtk::Box::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .build();
    let copy_button = gtk::Button::with_label("Copy");
    let paste_button = gtk::Button::with_label("Paste");
    let clear_button = gtk::Button::with_label("Clear");
    transcript_actions_row.append(&copy_button);
    transcript_actions_row.append(&paste_button);
    transcript_actions_row.append(&clear_button);
    recorder_page.append(&transcript_actions_row);

    let error_revealer = gtk::Revealer::new();
    error_revealer.set_reveal_child(false);
    error_revealer.set_transition_type(gtk::RevealerTransitionType::SlideDown);
    let error_box = gtk::Box::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .build();
    error_box.add_css_class("card");
    let error_label = gtk::Label::builder().xalign(0.0).hexpand(true).wrap(true).build();
    error_label.add_css_class("error");
    let open_settings_button = gtk::Button::with_label("Open Settings");
    let dismiss_error_button = gtk::Button::with_label("Dismiss");
    error_box.append(&error_label);
    error_box.append(&open_settings_button);
    error_box.append(&dismiss_error_button);
    error_revealer.set_child(Some(&error_box));
    recorder_page.append(&error_revealer);

    stack.add_titled(&recorder_page, Some("recorder"), "Recorder");

    let settings_page = adw::PreferencesPage::new();

    let settings_general_group = adw::PreferencesGroup::builder()
        .title("General")
        .description("Tune transcription and insertion behavior.")
        .build();

    let settings_autopaste_row = adw::ActionRow::builder()
        .title("Auto-paste")
        .subtitle("Automatically press Ctrl+V after transcription (optional: ydotool).")
        .build();
    let settings_autopaste_switch = gtk::Switch::builder()
        .active(config.borrow().autopaste)
        .valign(gtk::Align::Center)
        .build();
    settings_autopaste_switch.set_sensitive(autopaste_available);
    settings_autopaste_row.add_suffix(&settings_autopaste_switch);
    settings_general_group.add(&settings_autopaste_row);

    let model_entry = gtk::Entry::builder()
        .hexpand(true)
        .text(config.borrow().model_path.display().to_string())
        .placeholder_text("/home/user/.local/share/vdora/models/ggml-base.en.bin")
        .build();
    model_entry.set_width_chars(36);
    let model_row = adw::ActionRow::builder()
        .title("Model path")
        .subtitle("Local Whisper model file path")
        .build();
    model_row.add_suffix(&model_entry);
    settings_general_group.add(&model_row);

    let language_entry = gtk::Entry::builder()
        .hexpand(true)
        .text(config.borrow().language.clone().unwrap_or_default())
        .placeholder_text("auto (blank), en, es, ...")
        .build();
    language_entry.set_width_chars(16);
    let language_row = adw::ActionRow::builder()
        .title("Language")
        .subtitle("Leave blank for auto-detect")
        .build();
    language_row.add_suffix(&language_entry);
    settings_general_group.add(&language_row);

    let hotkey_entry = gtk::Entry::builder()
        .hexpand(true)
        .text(config.borrow().hotkey.clone())
        .placeholder_text("Ctrl+Alt+Space or <Ctrl><Alt>space")
        .build();
    hotkey_entry.set_width_chars(20);
    let hotkey_row = adw::ActionRow::builder()
        .title("Record toggle hotkey")
        .subtitle("App-level shortcut; global behavior depends on session")
        .build();
    hotkey_row.add_suffix(&hotkey_entry);
    settings_general_group.add(&hotkey_row);

    let log_level_options = gtk::StringList::new(&["info", "debug"]);
    let log_level_dropdown = gtk::DropDown::builder().model(&log_level_options).build();
    log_level_dropdown.set_selected(log_level_to_index(config.borrow().log_level));
    let log_level_row = adw::ActionRow::builder()
        .title("Log level")
        .subtitle("Applied on next launch (info or debug)")
        .build();
    log_level_row.add_suffix(&log_level_dropdown);
    settings_general_group.add(&log_level_row);

    settings_page.add(&settings_general_group);

    let settings_actions_group = adw::PreferencesGroup::builder()
        .title("Apply")
        .build();

    let save_button = gtk::Button::with_label("Save Settings");
    save_button.add_css_class("suggested-action");
    settings_actions_group.add(&save_button);

    let test_checks_button = gtk::Button::with_label("Test Checks Now");
    settings_actions_group.add(&test_checks_button);

    let settings_hint = gtk::Label::builder()
        .label(settings_hint_text())
        .css_classes(["dim-label"])
        .xalign(0.0)
        .wrap(true)
        .build();
    settings_actions_group.add(&settings_hint);
    settings_page.add(&settings_actions_group);

    stack.add_titled(&settings_page, Some("settings"), "Settings");

    let diagnostics_page = gtk::Box::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();

    let diagnostics_title = gtk::Label::builder()
        .label("Diagnostics")
        .css_classes(["title-2"])
        .xalign(0.0)
        .build();
    diagnostics_page.append(&diagnostics_title);

    let diagnostics_subtitle = gtk::Label::builder()
        .label("Dependency health, session details, and recent user-visible errors.")
        .css_classes(["dim-label"])
        .xalign(0.0)
        .wrap(true)
        .build();
    diagnostics_page.append(&diagnostics_subtitle);

    let health_pw_record_label = gtk::Label::builder().xalign(0.0).wrap(true).build();
    let health_wl_copy_label = gtk::Label::builder().xalign(0.0).wrap(true).build();
    let health_ydotool_label = gtk::Label::builder().xalign(0.0).wrap(true).build();
    let health_model_label = gtk::Label::builder().xalign(0.0).wrap(true).build();
    let health_session_label = gtk::Label::builder().xalign(0.0).wrap(true).build();

    diagnostics_page.append(&health_session_label);
    diagnostics_page.append(&health_pw_record_label);
    diagnostics_page.append(&health_wl_copy_label);
    diagnostics_page.append(&health_ydotool_label);
    diagnostics_page.append(&health_model_label);

    let last_error_title = gtk::Label::builder()
        .label("Last errors")
        .xalign(0.0)
        .build();
    diagnostics_page.append(&last_error_title);

    let last_error_view = gtk::TextView::builder()
        .editable(false)
        .cursor_visible(false)
        .wrap_mode(gtk::WrapMode::WordChar)
        .vexpand(true)
        .build();
    let last_error_buffer = gtk::TextBuffer::new(None);
    last_error_buffer.set_text("No recent errors");
    last_error_view.set_buffer(Some(&last_error_buffer));
    let last_error_scroll = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hexpand(true)
        .min_content_height(150)
        .child(&last_error_view)
        .build();
    diagnostics_page.append(&last_error_scroll);

    let diagnostics_actions_row = gtk::Box::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .build();
    let copy_diagnostics_button = gtk::Button::with_label("Copy Diagnostics");
    let export_diagnostics_button = gtk::Button::with_label("Export Log");
    let refresh_diagnostics_button = gtk::Button::with_label("Refresh Checks");
    diagnostics_actions_row.append(&copy_diagnostics_button);
    diagnostics_actions_row.append(&export_diagnostics_button);
    diagnostics_actions_row.append(&refresh_diagnostics_button);
    diagnostics_page.append(&diagnostics_actions_row);

    stack.add_titled(&diagnostics_page, Some("diagnostics"), "Diagnostics");

    let diagnostics_health = Rc::new(RefCell::new(diagnostics::collect_health(
        validate_model_path(&config.borrow().model_path).is_ok(),
    )));

    let refresh_diagnostics: Rc<dyn Fn()> = {
        let config = config.clone();
        let diagnostics_health = diagnostics_health.clone();
        let health_pw_record_label = health_pw_record_label.clone();
        let health_wl_copy_label = health_wl_copy_label.clone();
        let health_ydotool_label = health_ydotool_label.clone();
        let health_model_label = health_model_label.clone();
        let health_session_label = health_session_label.clone();
        let last_error_buffer = last_error_buffer.clone();
        let reporter = reporter.clone();

        Rc::new(move || {
            let model_ready = validate_model_path(&config.borrow().model_path).is_ok();
            let health = diagnostics::collect_health(model_ready);
            let mut health_state = diagnostics_health.borrow_mut();
            *health_state = health.clone();

            let lines = diagnostics::health_lines(&health);
            if let Some(line) = lines.first() {
                health_session_label.set_label(line);
            }
            if let Some(line) = lines.get(1) {
                health_pw_record_label.set_label(line);
            }
            if let Some(line) = lines.get(2) {
                health_wl_copy_label.set_label(line);
            }
            if let Some(line) = lines.get(3) {
                health_ydotool_label.set_label(line);
            }
            if let Some(line) = lines.get(4) {
                health_model_label.set_label(line);
            }

            let errors = reporter.snapshot();
            if errors.is_empty() {
                last_error_buffer.set_text("No recent errors");
            } else {
                let text = errors
                    .iter()
                    .rev()
                    .map(|e| format!("[{}] {}: {}", e.unix_seconds, e.source, e.message))
                    .collect::<Vec<_>>()
                    .join("\n");
                last_error_buffer.set_text(&text);
            }
        })
    };
    refresh_diagnostics();

    let (sender, receiver) = mpsc::channel::<AppMessage>();
    let (tray_sender, tray_receiver) = mpsc::channel::<TrayEvent>();
    let tray_controller = match crate::tray::spawn(tray_sender) {
        Ok(controller) => Some(controller),
        Err(err) => {
            tracing::warn!("failed to start tray service: {err}");
            None
        }
    };
    let tray_available = tray_controller.is_some();

    {
        let status = status.clone();
        let state = state.clone();
        let overlay = overlay.clone();
        let transcript_buffer = transcript_buffer.clone();
        let settings_autopaste_switch = settings_autopaste_switch.clone();
        let record_button = record_button.clone();
        let window = window.clone();
        let app = app.clone();
        let tray_controller = tray_controller.clone();
        let status_spinner = status_spinner.clone();
        let recording_started_at = recording_started_at.clone();
        let current_transcription_id = current_transcription_id.clone();
        let elapsed_label = elapsed_label.clone();
        let error_revealer = error_revealer.clone();
        let error_label = error_label.clone();
        let reporter = reporter.clone();
        let refresh_diagnostics = refresh_diagnostics.clone();

        glib::timeout_add_local(Duration::from_millis(250), move || {
            if *state.borrow() == AppState::Recording {
                if let Some(started_at) = *recording_started_at.borrow() {
                    let elapsed = started_at.elapsed().as_secs();
                    let minutes = elapsed / 60;
                    let seconds = elapsed % 60;
                    elapsed_label.set_label(&format!("Recording: {minutes:02}:{seconds:02}"));
                }
            } else {
                elapsed_label.set_label("");
            }

            while let Ok(event) = tray_receiver.try_recv() {
                match event {
                    TrayEvent::ShowWindow => {
                        window.present();
                    }
                    TrayEvent::HideWindow => {
                        window.set_visible(false);
                    }
                    TrayEvent::Quit => {
                        app.quit();
                    }
                }
            }

            while let Ok(message) = receiver.try_recv() {
                match message {
                    AppMessage::TranscriptionFinished { id, result } => {
                        let active_transcription_id = *current_transcription_id.borrow();
                        if should_ignore_transcription_result(active_transcription_id, id) {
                            tracing::debug!(
                                "ignoring stale transcription result with id {id}; active id: {:?}",
                                active_transcription_id
                            );
                            continue;
                        }

                        *current_transcription_id.borrow_mut() = None;
                        *state.borrow_mut() = AppState::Idle;
                        *recording_started_at.borrow_mut() = None;
                        status.set_label(AppState::Idle.label());
                        status_spinner.stop();
                        record_button.set_label("Start Recording");
                        record_button.set_sensitive(true);
                        if let Some(controller) = &tray_controller {
                            controller.set_status(AppState::Idle.label());
                        }

                        match result {
                            Ok(transcript) => {
                                error_revealer.set_reveal_child(false);
                                if transcript.trim().is_empty() {
                                    transcript_buffer.set_text("");
                                    add_toast(&overlay, "No speech detected. Ready when you are.");
                                    refresh_diagnostics();
                                    continue;
                                }

                                transcript_buffer.set_text(&transcript);

                                if let Err(err) = clipboard::set_text(&transcript) {
                                    add_error_toast(
                                        &reporter,
                                        &overlay,
                                        "clipboard",
                                        &format!("Clipboard error: {err}"),
                                    );
                                    refresh_diagnostics();
                                } else if settings_autopaste_switch.is_active() {
                                    if window.is_active() {
                                        match paste::trigger_ctrl_v() {
                                            Ok(()) => add_toast(&overlay, "Transcribed and pasted"),
                                            Err(err) => {
                                                add_error_toast(
                                                    &reporter,
                                                    &overlay,
                                                    "paste",
                                                    &format!(
                                                        "Copied only. Auto-paste unavailable: {err}"
                                                    ),
                                                );
                                                refresh_diagnostics();
                                            }
                                        }
                                    } else {
                                        add_toast(
                                            &overlay,
                                            "Copied only. Focus Vdora to allow auto-paste safely.",
                                        );
                                    }
                                } else {
                                    add_toast(&overlay, "Transcribed and copied")
                                }
                            }
                            Err(err) => {
                                transcript_buffer.set_text("");
                                *state.borrow_mut() = AppState::Error;
                                status.set_label(AppState::Error.label());
                                status_spinner.stop();
                                if let Some(controller) = &tray_controller {
                                    controller.set_status(AppState::Error.label());
                                }
                                error_label.set_label(&err);
                                error_revealer.set_reveal_child(true);
                                add_error_toast(&reporter, &overlay, "transcription", &err);
                                refresh_diagnostics();
                            }
                        }
                    }
                }
            }
            glib::ControlFlow::Continue
        });
    }

    refresh_diagnostics();
    if diagnostics_health.borrow().failure_count() > 0 {
        add_error_toast(
            &reporter,
            &overlay,
            "startup-diagnostics",
            "Diagnostics found missing required components. Open Diagnostics for details.",
        );
    }

    let toggle_recording: Rc<dyn Fn()> = {
        let status = status.clone();
        let overlay = overlay.clone();
        let sender = sender.clone();
        let config = config.clone();
        let recorder = recorder.clone();
        let active_session = active_session.clone();
        let recording_started_at = recording_started_at.clone();
        let next_transcription_id = next_transcription_id.clone();
        let current_transcription_id = current_transcription_id.clone();
        let state = state.clone();
        let record_button = record_button.clone();
        let tray_controller = tray_controller.clone();
        let status_spinner = status_spinner.clone();
        let error_revealer = error_revealer.clone();
        let reporter = reporter.clone();
        let refresh_diagnostics = refresh_diagnostics.clone();

        Rc::new(move || {
            if !can_toggle_recording(*state.borrow()) {
                add_toast(&overlay, "Please wait: transcription is still running.");
                return;
            }

            let currently_recording = active_session.borrow().is_some();

            if !currently_recording {
                error_revealer.set_reveal_child(false);
                match recorder.start() {
                    Ok(session) => {
                        *active_session.borrow_mut() = Some(session);
                        *recording_started_at.borrow_mut() = Some(Instant::now());
                        *state.borrow_mut() = AppState::Recording;
                        status.set_label(AppState::Recording.label());
                        status_spinner.start();
                        if let Some(controller) = &tray_controller {
                            controller.set_status(AppState::Recording.label());
                        }
                        record_button.set_label("Stop Recording");
                        add_toast(&overlay, "Listening...");
                    }
                    Err(err) => {
                        *recording_started_at.borrow_mut() = None;
                        *state.borrow_mut() = AppState::Error;
                        status.set_label(AppState::Error.label());
                        status_spinner.stop();
                        if let Some(controller) = &tray_controller {
                            controller.set_status(AppState::Error.label());
                        }
                        add_error_toast(
                            &reporter,
                            &overlay,
                            "recorder",
                            &format!("Unable to record: {err}"),
                        );
                        refresh_diagnostics();
                    }
                }
                return;
            }

            let Some(session) = active_session.borrow_mut().take() else {
                add_toast(&overlay, "No active recording session");
                return;
            };

            record_button.set_label("Start Recording");
            record_button.set_sensitive(false);
            *recording_started_at.borrow_mut() = None;
            *state.borrow_mut() = AppState::Transcribing;
            status.set_label(AppState::Transcribing.label());
            status_spinner.start();
            if let Some(controller) = &tray_controller {
                controller.set_status(AppState::Transcribing.label());
            }

            let sender = sender.clone();
            let transcription_id = {
                let mut next_id = next_transcription_id.borrow_mut();
                let id = *next_id;
                *next_id += 1;
                id
            };
            *current_transcription_id.borrow_mut() = Some(transcription_id);
            let config_snapshot = config.borrow().clone();
            thread::spawn(move || {
                let result = transcribe_session(session, config_snapshot)
                    .map_err(|err| format!("Transcription failed: {err:#}"));

                if sender
                    .send(AppMessage::TranscriptionFinished {
                        id: transcription_id,
                        result,
                    })
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
        add_error_toast(
            &reporter,
            &overlay,
            "settings",
            &format!("Invalid hotkey in config: {err}. Reset to default."),
        );
        let default = hotkey::default_hotkey().to_string();
        if let Err(reset_err) = apply_hotkey_accel(app, &default) {
            add_error_toast(
                &reporter,
                &overlay,
                "settings",
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
        let model_status_label = model_status_label.clone();
        let hotkey_label = hotkey_label.clone();
        let recorder_hint = recorder_hint.clone();
        let settings_autopaste_switch = settings_autopaste_switch.clone();
        let log_level_dropdown = log_level_dropdown.clone();
        let model_entry = model_entry.clone();
        let language_entry = language_entry.clone();
        let hotkey_entry = hotkey_entry.clone();
        let autopaste_available = autopaste_available;
        let reporter = reporter.clone();
        let refresh_diagnostics = refresh_diagnostics.clone();

        save_button.connect_clicked(move |_| {
            let autopaste_enabled = settings_autopaste_switch.is_active();
            let normalized_hotkey = match parse_hotkey_accel(&hotkey_entry.text()) {
                Ok(v) => v,
                Err(err) => {
                    add_error_toast(
                        &reporter,
                        &overlay,
                        "settings",
                        &format!("Invalid hotkey: {err}"),
                    );
                    refresh_diagnostics();
                    return;
                }
            };

            let next_cfg = match build_next_config(
                &config.borrow(),
                &model_entry.text(),
                &language_entry.text(),
                &normalized_hotkey,
                autopaste_enabled,
                autopaste_available,
            ) {
                Ok(cfg) => cfg,
                Err(err) => {
                    if autopaste_enabled && !autopaste_available {
                        settings_autopaste_switch.set_active(false);
                    }
                    add_error_toast(&reporter, &overlay, "settings", &err.to_string());
                    refresh_diagnostics();
                    return;
                }
            };

            let selected_log_level = match parse_log_level_selection(log_level_dropdown.selected()) {
                Some(level) => level,
                None => {
                    add_error_toast(
                        &reporter,
                        &overlay,
                        "settings",
                        "Invalid log level selection. Choose info or debug.",
                    );
                    refresh_diagnostics();
                    return;
                }
            };

            let mut next_cfg = next_cfg;
            next_cfg.log_level = selected_log_level;

            if let Err(err) = next_cfg.save() {
                add_error_toast(
                    &reporter,
                    &overlay,
                    "settings",
                    &format!("Failed to save settings: {err}"),
                );
                refresh_diagnostics();
                return;
            }

            app.set_accels_for_action("app.toggle-recording", &[&next_cfg.hotkey]);

            {
                let mut cfg = config.borrow_mut();
                *cfg = next_cfg;
            }

            model_label.set_label(&format!("Model: {}", config.borrow().model_path.display()));
            model_status_label.set_label(&format_model_status(config.borrow().model_path.is_file()));
            hotkey_label.set_label(&format_hotkey_line(&config.borrow()));
            recorder_hint.set_label(&format_hotkey_tip(&config.borrow()));
            settings_autopaste_switch.set_active(config.borrow().autopaste);
            log_level_dropdown.set_selected(log_level_to_index(config.borrow().log_level));

            add_toast(&overlay, "Settings saved");
            refresh_diagnostics();
        });
    }

    {
        let overlay = overlay.clone();
        let diagnostics_health = diagnostics_health.clone();
        let refresh_diagnostics = refresh_diagnostics.clone();
        test_checks_button.connect_clicked(move |_| {
            refresh_diagnostics();
            let health = diagnostics_health.borrow();
            if health.failure_count() == 0 {
                add_toast(&overlay, "Diagnostics checks passed");
            } else {
                add_toast(
                    &overlay,
                    "Diagnostics checks found missing required components",
                );
            }
        });
    }

    {
        let overlay = overlay.clone();
        let reporter = reporter.clone();
        let config = config.clone();
        let diagnostics_health = diagnostics_health.clone();
        let refresh_diagnostics = refresh_diagnostics.clone();
        copy_diagnostics_button.connect_clicked(move |_| {
            refresh_diagnostics();
            let bundle = diagnostics::diagnostics_bundle(
                &config.borrow(),
                &diagnostics_health.borrow(),
                &reporter.snapshot(),
            );
            match clipboard::set_text(&bundle) {
                Ok(()) => add_toast(&overlay, "Copied diagnostics bundle"),
                Err(err) => add_error_toast(
                    &reporter,
                    &overlay,
                    "diagnostics",
                    &format!("Clipboard error: {err}"),
                ),
            }
        });
    }

    {
        let overlay = overlay.clone();
        let reporter = reporter.clone();
        let config = config.clone();
        let diagnostics_health = diagnostics_health.clone();
        let refresh_diagnostics = refresh_diagnostics.clone();
        export_diagnostics_button.connect_clicked(move |_| {
            refresh_diagnostics();
            let bundle = diagnostics::diagnostics_bundle(
                &config.borrow(),
                &diagnostics_health.borrow(),
                &reporter.snapshot(),
            );
            match diagnostics::export_diagnostics_bundle(&bundle) {
                Ok(path) => add_toast(&overlay, &format!("Diagnostics exported: {}", path.display())),
                Err(err) => add_error_toast(
                    &reporter,
                    &overlay,
                    "diagnostics",
                    &format!("Failed to export diagnostics: {err}"),
                ),
            }
        });
    }

    {
        let refresh_diagnostics = refresh_diagnostics.clone();
        refresh_diagnostics_button.connect_clicked(move |_| {
            refresh_diagnostics();
        });
    }

    {
        let overlay = overlay.clone();
        let transcript_buffer = transcript_buffer.clone();
        let reporter = reporter.clone();
        let refresh_diagnostics = refresh_diagnostics.clone();
        copy_button.connect_clicked(move |_| {
            let start = transcript_buffer.start_iter();
            let end = transcript_buffer.end_iter();
            let text = transcript_buffer.text(&start, &end, false).to_string();
            if transcript_is_empty(&text) {
                add_toast(&overlay, "Nothing to copy yet");
                return;
            }
            if let Err(err) = clipboard::set_text(&text) {
                add_error_toast(
                    &reporter,
                    &overlay,
                    "clipboard",
                    &format!("Clipboard error: {err}"),
                );
                refresh_diagnostics();
            } else {
                add_toast(&overlay, "Copied transcript");
            }
        });
    }

    {
        let overlay = overlay.clone();
        let transcript_buffer = transcript_buffer.clone();
        let reporter = reporter.clone();
        let refresh_diagnostics = refresh_diagnostics.clone();
        paste_button.connect_clicked(move |_| {
            let start = transcript_buffer.start_iter();
            let end = transcript_buffer.end_iter();
            let text = transcript_buffer.text(&start, &end, false).to_string();
            if transcript_is_empty(&text) {
                add_toast(&overlay, "Nothing to paste yet");
                return;
            }

            if let Err(err) = clipboard::set_text(&text) {
                add_error_toast(
                    &reporter,
                    &overlay,
                    "clipboard",
                    &format!("Clipboard error: {err}"),
                );
                refresh_diagnostics();
                return;
            }

            match paste::trigger_ctrl_v() {
                Ok(()) => add_toast(&overlay, "Pasted transcript"),
                Err(err) => {
                    add_error_toast(
                        &reporter,
                        &overlay,
                        "paste",
                        &format!("Copied only. Auto-paste unavailable: {err}"),
                    );
                    refresh_diagnostics();
                }
            }
        });
    }

    {
        let transcript_buffer = transcript_buffer.clone();
        clear_button.connect_clicked(move |_| {
            transcript_buffer.set_text("");
        });
    }

    {
        let stack = stack.clone();
        open_settings_button.connect_clicked(move |_| {
            stack.set_visible_child_name("settings");
        });
    }

    {
        let error_revealer = error_revealer.clone();
        dismiss_error_button.connect_clicked(move |_| {
            error_revealer.set_reveal_child(false);
        });
    }

    if should_hide_on_close(tray_available) {
        window.connect_close_request(|window| {
            window.set_visible(false);
            glib::Propagation::Stop
        });
    }

    if should_start_hidden(tray_available) {
        window.set_visible(false);
    } else {
        window.present();
    }
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

fn can_toggle_recording(state: AppState) -> bool {
    state != AppState::Transcribing
}

fn should_ignore_transcription_result(active_id: Option<u64>, incoming_id: u64) -> bool {
    active_id != Some(incoming_id)
}

fn transcript_is_empty(text: &str) -> bool {
    text.trim().is_empty()
}

fn parse_log_level_selection(selected_index: u32) -> Option<LogLevel> {
    match selected_index {
        0 => Some(LogLevel::Info),
        1 => Some(LogLevel::Debug),
        _ => None,
    }
}

fn log_level_to_index(log_level: LogLevel) -> u32 {
    match log_level {
        LogLevel::Info => 0,
        LogLevel::Debug => 1,
    }
}

fn should_start_hidden(tray_available: bool) -> bool {
    tray_available
}

fn should_hide_on_close(tray_available: bool) -> bool {
    tray_available
}

fn build_next_config(
    current: &AppConfig,
    model_text: &str,
    language_text: &str,
    normalized_hotkey: &str,
    autopaste_enabled: bool,
    autopaste_available: bool,
) -> Result<AppConfig> {
    let model_path_text = model_text.trim();
    if model_path_text.is_empty() {
        return Err(anyhow!("Model path cannot be empty"));
    }

    let model_path = PathBuf::from(model_path_text);
    if let Err(err) = validate_model_path(&model_path) {
        return Err(anyhow!("Invalid model path: {err}"));
    }

    if autopaste_enabled && !autopaste_available {
        return Err(anyhow!(
            "Auto-paste requires ydotool/ydotoold. Install it or keep auto-paste disabled."
        ));
    }

    let mut next_cfg = current.clone();
    next_cfg.model_path = model_path;
    let language = language_text.trim();
    next_cfg.language = if language.is_empty() {
        None
    } else {
        Some(language.to_string())
    };
    next_cfg.autopaste = autopaste_enabled;
    next_cfg.hotkey = normalized_hotkey.trim().to_string();

    Ok(next_cfg)
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

fn format_hotkey_tip(config: &AppConfig) -> String {
    format!("Tip: use {} to start or stop recording quickly.", config.hotkey)
}

fn format_model_status(is_ready: bool) -> &'static str {
    if is_ready {
        "Model status: ready"
    } else {
        "Model status: missing file (open Settings to fix model path)"
    }
}

fn format_autopaste_status(is_available: bool) -> &'static str {
    if is_available {
        "Auto-paste dependency: available"
    } else {
        "Auto-paste dependency: unavailable (install ydotool/ydotoold)"
    }
}

fn settings_hint_text() -> &'static str {
    "Hotkey accepts forms like Ctrl+Alt+Space or <Ctrl><Alt>space. Auto-paste is optional and requires ydotool/ydotoold. On Wayland this hotkey works while the app is focused; use GNOME custom shortcuts for global behavior."
}

fn add_toast(overlay: &adw::ToastOverlay, text: &str) {
    let toast = adw::Toast::new(text);
    overlay.add_toast(toast);
}

fn add_error_toast(
    reporter: &diagnostics::Reporter,
    overlay: &adw::ToastOverlay,
    source: &str,
    text: &str,
) {
    reporter.record_error(source, text);
    add_toast(overlay, text);
}

fn transcribe_session(session: RecordingSession, config: AppConfig) -> Result<String> {
    let recording = session.stop().context("failed to finish audio recording")?;
    transcribe_file(recording.path(), &config)
}

fn validate_model_path(path: &PathBuf) -> Result<()> {
    let metadata = fs::metadata(path)
        .with_context(|| format!("failed to read model metadata at {}", path.display()))?;
    if !metadata.is_file() {
        return Err(anyhow!("model path must point to a regular file"));
    }
    Ok(())
}

fn transcribe_file(wav_path: &std::path::Path, config: &AppConfig) -> Result<String> {
    let whisper = WhisperService::new(config.model_path.clone(), config.language.clone());
    whisper.transcribe_file(wav_path)
}

#[cfg(test)]
mod tests {
    use super::{
        build_next_config, can_toggle_recording, parse_log_level_selection,
        should_hide_on_close, should_ignore_transcription_result, should_start_hidden,
        transcript_is_empty,
    };
    use crate::config::{AppConfig, LogLevel};
    use crate::state::AppState;
    use tempfile::NamedTempFile;

    #[test]
    fn toggle_recording_blocked_while_transcribing() {
        assert!(!can_toggle_recording(AppState::Transcribing));
        assert!(can_toggle_recording(AppState::Idle));
        assert!(can_toggle_recording(AppState::Recording));
        assert!(can_toggle_recording(AppState::Error));
    }

    #[test]
    fn stale_transcription_ids_are_ignored() {
        assert!(should_ignore_transcription_result(Some(3), 2));
        assert!(should_ignore_transcription_result(None, 1));
        assert!(!should_ignore_transcription_result(Some(7), 7));
    }

    #[test]
    fn transcript_empty_check_uses_trimmed_content() {
        assert!(transcript_is_empty(""));
        assert!(transcript_is_empty("   \n\t"));
        assert!(!transcript_is_empty("hello"));
        assert!(!transcript_is_empty("  hello  "));
    }

    #[test]
    fn build_next_config_updates_values_without_mutating_original() {
        let model = NamedTempFile::new().expect("temp model file should be created");
        let model_path = model.path().display().to_string();
        let current = AppConfig::default();

        let next = build_next_config(
            &current,
            &model_path,
            "en",
            "<Ctrl><Shift>r",
            true,
            true,
        )
        .expect("next config should build");

        assert!(!current.autopaste);
        assert_eq!(next.model_path, model.path());
        assert_eq!(next.language.as_deref(), Some("en"));
        assert!(next.autopaste);
        assert_eq!(next.hotkey, "<Ctrl><Shift>r");
    }

    #[test]
    fn build_next_config_rejects_unavailable_autopaste() {
        let model = NamedTempFile::new().expect("temp model file should be created");
        let model_path = model.path().display().to_string();
        let current = AppConfig::default();

        let err = build_next_config(&current, &model_path, "", "<Ctrl><Alt>space", true, false)
            .expect_err("autopaste should require availability");

        assert!(
            err.to_string()
                .contains("Auto-paste requires ydotool/ydotoold"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn build_next_config_rejects_empty_model_path() {
        let current = AppConfig::default();
        let err = build_next_config(&current, "   ", "", "<Ctrl><Alt>space", false, true)
            .expect_err("empty model path should be rejected");
        assert!(err.to_string().contains("Model path cannot be empty"));
    }

    #[test]
    fn build_next_config_keeps_existing_log_level() {
        let model = NamedTempFile::new().expect("temp model file should be created");
        let model_path = model.path().display().to_string();
        let mut current = AppConfig::default();
        current.log_level = LogLevel::Debug;

        let next = build_next_config(
            &current,
            &model_path,
            "",
            "<Ctrl><Alt>space",
            false,
            true,
        )
        .expect("next config should build");

        assert_eq!(next.log_level, LogLevel::Debug);
    }

    #[test]
    fn parses_log_level_selection() {
        assert_eq!(parse_log_level_selection(0), Some(LogLevel::Info));
        assert_eq!(parse_log_level_selection(1), Some(LogLevel::Debug));
        assert_eq!(parse_log_level_selection(3), None);
    }

    #[test]
    fn startup_visibility_depends_on_tray_availability() {
        assert!(should_start_hidden(true));
        assert!(!should_start_hidden(false));
    }

    #[test]
    fn close_behavior_depends_on_tray_availability() {
        assert!(should_hide_on_close(true));
        assert!(!should_hide_on_close(false));
    }
}
