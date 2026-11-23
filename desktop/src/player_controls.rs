use std::cell::RefCell;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use slint::ComponentHandle;

use crate::MainWindow;
use crate::backend;
use crate::config;
use crate::models::PlaybackSyncState;

type PlayerControlRequest = (String, Option<f64>);

pub(crate) fn start_player_control_worker(api_base_url: String) -> Sender<PlayerControlRequest> {
    let (sender, receiver) = mpsc::channel::<PlayerControlRequest>();

    thread::spawn(move || {
        let client = match reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
        {
            Ok(client) => client,
            Err(error) => {
                eprintln!("[Player] Failed to create HTTP client: {error}");
                return;
            }
        };

        let endpoint = format!("{}/api/player/control", api_base_url.trim_end_matches('/'));

        while let Ok((action, value)) = receiver.recv() {
            let payload = match value {
                Some(value) => serde_json::json!({
                    "action": action,
                    "value": value
                }),
                None => serde_json::json!({
                    "action": action
                }),
            };

            let result = client
                .post(&endpoint)
                .header("Content-Type", "application/json")
                .body(payload.to_string())
                .send();

            match result {
                Ok(response) => {
                    let status = response.status();

                    match response.text() {
                        Ok(body) => {
                            println!(
                                "[Player] Control response: \
                                 action={action}, status={status}, body={body}"
                            );

                            if !status.is_success() {
                                eprintln!(
                                    "[Player] Backend rejected {action}: \
                                     HTTP {status}"
                                );
                            }
                        }
                        Err(error) => {
                            eprintln!("[Player] Failed to read {action} response: {error}");
                        }
                    }
                }
                Err(error) => {
                    eprintln!("[Player] Failed to send {action} control: {error}");
                }
            }
        }
    });

    sender
}

pub(crate) fn send_player_control(
    sender: &Sender<PlayerControlRequest>,
    action: &str,
    value: Option<f64>,
) {
    if let Err(error) = sender.send((action.to_owned(), value)) {
        eprintln!("[Player] Failed to queue control request: {error}");
    }
}

pub(crate) fn setup_playback_controls(
    ui: &MainWindow,
    api_base_url: String,
    control_sender: Sender<PlayerControlRequest>,
    preferred_volume: Arc<Mutex<f64>>,
    playback_sync: Arc<Mutex<PlaybackSyncState>>,
    config: std::rc::Rc<RefCell<config::AppConfig>>,
) {
    ui.on_skip_requested(move || {
        backend::skip(api_base_url.clone());
    });

    let sender = control_sender.clone();

    ui.on_seek_requested(move |position| {
        send_player_control(&sender, "seek", Some(f64::from(position)));
    });

    let sender = control_sender.clone();
    let weak_ui = ui.as_weak();

    ui.on_volume_requested(move |volume| {
        let volume = f64::from(volume).clamp(0.0, 1.0);

        if let Some(ui) = weak_ui.upgrade() {
            ui.set_playback_volume(volume as f32);
        }

        match preferred_volume.lock() {
            Ok(mut preferred_volume) => *preferred_volume = volume,
            Err(error) => {
                eprintln!("[Config] Failed to update playback volume: {error}");
            }
        }

        match playback_sync.lock() {
            Ok(mut sync) => sync.playback.volume = volume,
            Err(error) => {
                eprintln!("[Player] Failed to update synchronized volume: {error}");
            }
        }

        {
            let mut app_config = config.borrow_mut();
            app_config.playback.volume = volume;

            if let Err(error) = app_config.save() {
                eprintln!("[Config] Failed to save playback volume: {error}");
            }
        }

        send_player_control(&sender, "volume", Some(volume));
    });

    let sender = control_sender;
    let weak_ui = ui.as_weak();

    ui.on_playback_toggle_requested(move || {
        let Some(ui) = weak_ui.upgrade() else {
            return;
        };

        if ui.get_current_media_url().is_empty() {
            return;
        }

        let action = if ui.get_playback_paused() {
            "play"
        } else {
            "pause"
        };

        send_player_control(&sender, action, None);
    });
}
