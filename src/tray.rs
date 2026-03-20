use std::sync::mpsc::Sender;
use std::{env, path::PathBuf};

use anyhow::Result;
use ksni::menu::StandardItem;
use ksni::ToolTip;

#[derive(Debug, Clone, Copy)]
pub enum TrayEvent {
    ShowWindow,
    HideWindow,
    ToggleRecording,
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
        });
    }
}

pub fn spawn(sender: Sender<TrayEvent>) -> Result<TrayController> {
    let tray = VdoraTray {
        sender,
        status: "Idle".to_string(),
        icon_name: resolve_icon_name(),
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
                label: "Show Window".into(),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.sender.send(TrayEvent::ShowWindow);
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Hide Window".into(),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.sender.send(TrayEvent::HideWindow);
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Start/Stop Recording".into(),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.sender.send(TrayEvent::ToggleRecording);
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Quit".into(),
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
