use std::sync::mpsc::Sender;
use std::{env, path::PathBuf};

use anyhow::Result;
use ksni::menu::StandardItem;
use ksni::ToolTip;

const MENU_SHOW_WINDOW: &str = "Show Window";
const MENU_HIDE_WINDOW: &str = "Hide Window";
const MENU_QUIT: &str = "Quit";

#[derive(Debug, Clone, Copy)]
pub enum TrayEvent {
    ShowWindow,
    HideWindow,
    Quit,
}

#[derive(Clone)]
pub struct TrayController {
    handle: ksni::Handle<VdoraTray>,
}

impl TrayController {
    pub fn set_status(&self, status: &str) {
        self.handle.update(|tray| {
            tray.status = status.to_string();
            tray.icon_name = icon_name_for_status(&tray.base_icon_name, status);
        });
    }
}

pub fn spawn(sender: Sender<TrayEvent>) -> Result<TrayController> {
    let base_icon_name = resolve_icon_name();
    let tray = VdoraTray {
        sender,
        status: "Idle".to_string(),
        icon_name: base_icon_name.clone(),
        base_icon_name,
    };
    let service = ksni::TrayService::new(tray);
    let handle = service.handle();
    service.spawn();
    Ok(TrayController { handle })
}

struct VdoraTray {
    sender: Sender<TrayEvent>,
    status: String,
    icon_name: String,
    base_icon_name: String,
}

impl ksni::Tray for VdoraTray {
    fn id(&self) -> String {
        "com.vdora.App".to_string()
    }

    fn title(&self) -> String {
        "Vdora".to_string()
    }

    fn icon_name(&self) -> String {
        self.icon_name.clone()
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            icon_name: self.icon_name.clone(),
            title: "Vdora".to_string(),
            description: format!("Status: {}", self.status),
            ..Default::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.sender.send(TrayEvent::ShowWindow);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        vec![
            StandardItem {
                label: MENU_SHOW_WINDOW.into(),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.sender.send(TrayEvent::ShowWindow);
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: MENU_HIDE_WINDOW.into(),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.sender.send(TrayEvent::HideWindow);
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: MENU_QUIT.into(),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.sender.send(TrayEvent::Quit);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

fn resolve_icon_name() -> String {
    if app_icon_exists() {
        "vdora".to_string()
    } else {
        "audio-input-microphone-symbolic".to_string()
    }
}

fn app_icon_exists() -> bool {
    let mut roots = vec![PathBuf::from("/usr/share/icons"), PathBuf::from("/usr/local/share/icons")];
    if let Ok(home) = env::var("HOME") {
        roots.push(PathBuf::from(home).join(".local/share/icons"));
    }

    let candidates = [
        "hicolor/scalable/apps/vdora.svg",
        "hicolor/256x256/apps/vdora.png",
        "hicolor/128x128/apps/vdora.png",
        "hicolor/64x64/apps/vdora.png",
        "hicolor/48x48/apps/vdora.png",
        "hicolor/32x32/apps/vdora.png",
        "hicolor/24x24/apps/vdora.png",
        "hicolor/16x16/apps/vdora.png",
    ];

    for root in roots {
        for rel in candidates {
            if root.join(rel).is_file() {
                return true;
            }
        }
    }

    false
}

fn icon_name_for_status(base_icon_name: &str, status: &str) -> String {
    if status.starts_with("Recording") {
        "media-record-symbolic".to_string()
    } else {
        base_icon_name.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{icon_name_for_status, MENU_HIDE_WINDOW, MENU_QUIT, MENU_SHOW_WINDOW};

    #[test]
    fn tray_menu_labels_do_not_expose_record_toggle() {
        let labels = [MENU_SHOW_WINDOW, MENU_HIDE_WINDOW, MENU_QUIT];
        assert!(labels.contains(&"Show Window"));
        assert!(labels.contains(&"Hide Window"));
        assert!(labels.contains(&"Quit"));
        assert!(!labels.iter().any(|label| label.contains("Recording")));
    }

    #[test]
    fn tray_icon_switches_to_record_icon_when_recording() {
        assert_eq!(
            icon_name_for_status("vdora", "Recording..."),
            "media-record-symbolic"
        );
        assert_eq!(icon_name_for_status("vdora", "Idle"), "vdora");
        assert_eq!(
            icon_name_for_status("audio-input-microphone-symbolic", "Transcribing..."),
            "audio-input-microphone-symbolic"
        );
    }
}
