use log::error;
use std::{iter::once, process::Command, sync::Arc};

use ksni::{
    MenuItem,
    menu::{RadioGroup, RadioItem, StandardItem, SubMenu},
};
use tokio::sync::{RwLock, mpsc::Sender};

use crate::{
    ActionEvent,
    config::{Config, Container, Quality},
    kdialog::MessageBox,
    utils::ask_custom_number,
};

pub struct TrayIcon {
    _enabled: bool,
    tray_event_tx: Sender<ActionEvent>,
    config: Arc<RwLock<Config>>,
}

impl TrayIcon {
    pub async fn new(tray_event_tx: Sender<ActionEvent>, config: &Arc<RwLock<Config>>) -> Self {
        Self {
            tray_event_tx,
            _enabled: true,
            config: config.clone(),
        }
    }
}

struct TrayMultipleOption<T>(String, T);

impl<T> Into<RadioItem> for &TrayMultipleOption<T> {
    fn into(self) -> RadioItem {
        RadioItem {
            label: self.0.clone(),
            ..Default::default()
        }
    }
}

#[allow(dead_code)]
enum TrayConfigItem<T, O>
where
    T: ksni::Tray + CommunicationProvider,
{
    Multiple {
        label: String,
        icon: String,
        options: Vec<TrayMultipleOption<O>>,
        initial_state: usize,
        show_custom: bool,
        action: Box<dyn Fn(&mut T, usize) + Send + 'static>,
    },
    Toggle {
        label: String,
        icon: String,
        action: Box<dyn Fn(&mut T) + Send + 'static>,
    },
    Custom {
        label: String,
        icon: String,
        action: Box<dyn Fn(&mut T) + Send + 'static>,
    },
}

impl<T, O> Into<MenuItem<T>> for TrayConfigItem<T, O>
where
    T: ksni::Tray + CommunicationProvider,
{
    fn into(self) -> MenuItem<T> {
        match self {
            TrayConfigItem::Multiple {
                label,
                icon,
                options,
                action,
                initial_state,
                show_custom,
            } => SubMenu {
                label,
                icon_name: icon,
                submenu: vec![
                    RadioGroup {
                        selected: initial_state,
                        select: action,
                        options: {
                            let options = options.iter().map(|option| option.into());

                            if show_custom {
                                options
                                    .chain(once(RadioItem {
                                        label: "Custom...".into(),
                                        ..Default::default()
                                    }))
                                    .collect()
                            } else {
                                options.collect()
                            }
                        },
                    }
                    .into(),
                ],
                ..Default::default()
            }
            .into(),
            TrayConfigItem::Toggle {
                label: _,
                icon: _,
                action: _,
            } => todo!("Implement toggle config menu item type"),
            TrayConfigItem::Custom {
                label,
                icon,
                action,
            } => StandardItem {
                label,
                icon_name: icon,
                activate: action,
                ..Default::default()
            }
            .into(),
        }
    }
}

macro_rules! tray_config_item_radio {
    (@custombool nocustom) => { false };
    (@custombool) => { true };

    (@customhandler $config:expr, $config_key:ident, $label:expr, nocustom) => {};

    (@customhandler $config:expr, $config_key:ident, $label:expr,) => {
        match ask_custom_number("TrayPlay Settings", $label, 0) {
            Ok(number) => {
                if let Some(number) = number {
                    $config.$config_key = number;
                    $config.save().await;
                }
            }
            Err(err) => {
                error!("Error when asking for custom config value: {}", err);
            }
        }
    };

    ($config_key:ident, $config:expr, $label:expr, $icon:expr, $values:expr $(, $nocustom:tt)?) => {{
        let config = $config;

        TrayConfigItem::Multiple::<TrayIcon, _> {
            label: $label.into(),
            icon: $icon.into(),
            options: $values,
            show_custom: tray_config_item_radio!(@custombool $($nocustom)?),
            initial_state: $values
                .iter()
                .position(|element: &TrayMultipleOption<_>| {
                    let a = element.1;
                    a == config.$config_key
                })
                .unwrap_or($values.len()),
            action: Box::new(|item, selection| {
                futures::executor::block_on(async {
                    let config = item.get_config();
                    let mut config = config.write().await;
                    if selection >= $values.len() {
                        tray_config_item_radio!(@customhandler config, $config_key, $label, $($nocustom)?);
                    } else {
                        let values: Vec<TrayMultipleOption<_>> = $values;
                        config.$config_key = values[selection].1;
                        config.save().await;
                    }
                });
            }),
        }
    }};
}

macro_rules! tray_config_item_custom {
    ($label:expr, $icon:expr, $action:expr) => {
        TrayConfigItem::Custom::<TrayIcon, u8> {
            label: $label.into(),
            icon: $icon.into(),
            action: Box::new(|item| {
                futures::executor::block_on(async {
                    $action(item.get_config(), item.get_action_event_tx()).await;
                });
            }),
        }
    };
}

impl ksni::Tray for TrayIcon {
    const MENU_ON_ACTIVATE: bool = true;

    fn id(&self) -> String {
        env!("CARGO_PKG_NAME").into()
    }

    fn icon_name(&self) -> String {
        "media-skip-backward".into()
    }

    fn title(&self) -> String {
        "TrayPlay".into()
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        let tx_clone = self.tray_event_tx.clone();
        use ksni::menu::*;

        let config = futures::executor::block_on(async { self.config.read().await });

        let settings_menu = vec![
            tray_config_item_radio!(
                framerate,
                &config,
                "Framerate",
                "speedometer",
                vec![
                    TrayMultipleOption("30".into(), 30),
                    TrayMultipleOption("60".into(), 60),
                ]
            )
            .into(),
            tray_config_item_radio!(
                replay_duration_secs,
                &config,
                "Duration",
                "clock",
                vec![
                    TrayMultipleOption("30s".into(), 30),
                    TrayMultipleOption("1min".into(), 60),
                    TrayMultipleOption("2min".into(), 120),
                    TrayMultipleOption("3min".into(), 180),
                    TrayMultipleOption("5min".into(), 300),
                ]
            )
            .into(),
            tray_config_item_radio!(
                quality,
                &config,
                "Quality",
                "star-new-symbolic",
                vec![
                    TrayMultipleOption("Medium".into(), Quality::Medium),
                    TrayMultipleOption("High".into(), Quality::High),
                    TrayMultipleOption("Very high".into(), Quality::VeryHigh),
                    TrayMultipleOption("Ultra".into(), Quality::Ultra),
                ],
                nocustom
            )
            .into(),
            tray_config_item_radio!(
                container,
                &config,
                "Container",
                "archive-extract",
                vec![
                    TrayMultipleOption("MKV".into(), Container::MKV),
                    TrayMultipleOption("MP4".into(), Container::MP4),
                    TrayMultipleOption("WEBM".into(), Container::WEBM),
                    TrayMultipleOption("FLV".into(), Container::FLV),
                ],
                nocustom
            )
            .into(),
            tray_config_item_custom!(
                "Path",
                "inode-directory",
                async move |_, action_event_tx: Sender<ActionEvent>| {
                    // Need to send message to main thread because for some reason portal file picker request
                    // is not being sent when directly called here...
                    action_event_tx
                        .send(ActionEvent::ChangeReplayPath)
                        .await
                        .unwrap();
                }
            )
            .into(),
        ];

        vec![
            // TODO: implement toggling replays on and off
            // CheckmarkItem {
            //     label: "Record replays".into(),
            //     checked: self.enabled,
            //     icon_name: "media-skip-backward".into(),
            //     activate: Box::new(move |this: &mut Self| {
            //         this.enabled = !this.enabled;
            //         futures::executor::block_on(async {
            //             sender_clone1.send("toggle-replay".into()).await.unwrap();
            //         });
            //     }),
            //     ..Default::default()
            // }
            // .into(),
            StandardItem {
                label: "Save replay".into(),
                icon_name: "document-save".into(),
                activate: Box::new({
                    let tx_clone = tx_clone.clone();
                    move |_| {
                        futures::executor::block_on(async {
                            tx_clone.send(ActionEvent::SaveReplay).await.unwrap();
                        });
                    }
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            SubMenu {
                label: "Settings".into(),
                icon_name: "configure".into(),
                submenu: settings_menu,
                ..Default::default()
            }
            .into(),
            tray_config_item_custom!("About", "help-about", async move |_, _| {
                let gsr_version = Command::new("gpu-screen-recorder")
                    .arg("--version")
                    .output()
                    .unwrap();
                MessageBox::new(format!(
                    "TrayPlay version: {}\ngpu-screen-recorder version: {}\nReport issues at: https://github.com/kabuspl/trayplay/issues\nLicense: MIT\n© 2025 kabuspl",
                    env!("CARGO_PKG_VERSION"),
                    String::from_utf8(gsr_version.stdout).unwrap()
                ))
                .title("About TrayPlay")
                .show()
                .unwrap();
            })
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                icon_name: "gtk-quit".into(),
                activate: Box::new({
                    let tx_clone = tx_clone.clone();
                    move |_| {
                        futures::executor::block_on(async {
                            tx_clone.send(ActionEvent::Quit).await.unwrap();
                        });
                    }
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

impl CommunicationProvider for TrayIcon {
    fn get_config(&self) -> Arc<RwLock<Config>> {
        self.config.clone()
    }

    fn get_action_event_tx(&self) -> Sender<ActionEvent> {
        self.tray_event_tx.clone()
    }
}

trait CommunicationProvider {
    fn get_config(&self) -> Arc<RwLock<Config>>;
    fn get_action_event_tx(&self) -> Sender<ActionEvent>;
}
