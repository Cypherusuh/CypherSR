use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use arboard::Clipboard;
use open::that;
use slint::{ComponentHandle, ModelRc, SharedString, Timer, VecModel};

use crate::MainWindow;
use crate::backend;
use crate::models::PlaybackSyncState;
use crate::video_preview::{VideoPreviewController, native_window_handle};

const VIDEO_SYNC_INTERVAL: Duration = Duration::from_millis(100);

pub(crate) fn setup_window_controls(
    ui: &MainWindow,
    video_controller: Rc<RefCell<VideoPreviewController>>,
) {
    let weak_ui = ui.as_weak();

    ui.on_minimize_requested(move || {
        if let Some(ui) = weak_ui.upgrade() {
            ui.window().set_minimized(true);
        }
    });

    ui.on_close_window_requested(move || {
        video_controller.borrow_mut().shutdown();
        let _ = slint::quit_event_loop();
    });
}

pub(crate) fn setup_overlay_actions(ui: &MainWindow, api_base_url: String) {
    ui.on_copy_overlay_url(|url| match Clipboard::new() {
        Ok(mut clipboard) => match clipboard.set_text(url.to_string()) {
            Ok(()) => true,
            Err(error) => {
                eprintln!("[Overlay] Failed to copy URL: {error}");
                false
            }
        },
        Err(error) => {
            eprintln!("[Overlay] Failed to access clipboard: {error}");
            false
        }
    });

    ui.on_open_creator_link(|| {
        if let Err(error) = that("https://www.twitch.tv/cypherusuh__") {
            eprintln!("[Twitch] Failed to open creator link: {error}");
        }
    });

    let weak_ui = ui.as_weak();

    ui.on_connection_toggle_requested(move || {
        let Some(ui) = weak_ui.upgrade() else {
            return;
        };

        if ui.get_twitch_connecting() {
            return;
        }

        let should_connect = !ui.get_twitch_connected();

        backend::request_twitch_connection(api_base_url.clone(), should_connect);
    });
}

pub(crate) fn start_backend_polling(
    ui: &MainWindow,
    playback_sync: Arc<Mutex<PlaybackSyncState>>,
    api_base_url: String,
    preferred_volume: Arc<Mutex<f64>>,
) {
    let weak_ui = ui.as_weak();
    let playback_sync_for_volume = playback_sync.clone();

    backend::start_polling(api_base_url, playback_sync, move |snapshot| {
        let preferred_volume = match preferred_volume.lock() {
            Ok(volume) => *volume,
            Err(error) => {
                eprintln!("[Backend] Playback volume mutex poisoned: {error}");
                return;
            }
        };
        let mut snapshot = snapshot;
        snapshot.playback.volume = preferred_volume;

        match playback_sync_for_volume.lock() {
            Ok(mut sync) => sync.playback.volume = preferred_volume,
            Err(error) => {
                eprintln!(
                    "[Backend] Playback sync mutex poisoned while applying configured volume: {error}"
                );
                return;
            }
        }

        let weak_ui = weak_ui.clone();

        let result = slint::invoke_from_event_loop(move || {
            let Some(ui) = weak_ui.upgrade() else {
                return;
            };

            ui.set_connected(snapshot.connected);
            ui.set_current_track(snapshot.current_track.into());
            ui.set_current_artist(snapshot.current_artist.into());
            ui.set_current_media_url(snapshot.current_media_url.into());

            let items: Vec<SharedString> = snapshot
                .queue_items
                .into_iter()
                .map(SharedString::from)
                .collect();

            let model = Rc::new(VecModel::from(items));
            ui.set_queue_items(ModelRc::from(model));

            ui.set_playback_position(snapshot.playback.position as f32);
            ui.set_playback_duration(snapshot.playback.duration as f32);
            ui.set_playback_volume(snapshot.playback.volume as f32);
            ui.set_playback_paused(snapshot.playback.paused);
        });

        if let Err(error) = result {
            eprintln!("[Backend] Failed to schedule UI update: {error}");
        }
    });
}

pub(crate) fn start_twitch_status_polling(ui: &MainWindow, api_base_url: String) {
    let weak_ui = ui.as_weak();

    backend::start_twitch_status_polling(api_base_url, move |status| {
        let weak_ui = weak_ui.clone();

        let result = slint::invoke_from_event_loop(move || {
            let Some(ui) = weak_ui.upgrade() else {
                return;
            };

            let status_label = if status.reconnect_failed {
                "reconnect-failed"
            } else if status.reconnecting {
                "reconnecting"
            } else if status.error.is_some() {
                "error"
            } else if status.connecting {
                "connecting"
            } else if status.connected {
                "connected"
            } else {
                "offline"
            };

            ui.set_twitch_connected(status.connected);
            ui.set_twitch_connecting(
                status.connecting || (status.reconnecting && !status.reconnect_failed),
            );
            ui.set_twitch_reconnecting(status.reconnecting);
            ui.set_twitch_reconnect_failed(status.reconnect_failed);
            ui.set_twitch_status(status_label.into());

            if let Some(username) = status.username {
                if !username.trim().is_empty() {
                    ui.set_username(username.into());
                }
            }
        });

        if let Err(error) = result {
            eprintln!("[Twitch] Failed to schedule UI update: {error}");
        }
    });
}

pub(crate) fn setup_video_preview(
    ui: &MainWindow,
    video_controller: Rc<RefCell<VideoPreviewController>>,
    playback_sync: Arc<Mutex<PlaybackSyncState>>,
) -> Timer {
    let weak_ui = ui.as_weak();
    let video_timer = Timer::default();

    video_timer.start(slint::TimerMode::Repeated, VIDEO_SYNC_INTERVAL, move || {
        let Some(ui) = weak_ui.upgrade() else {
            video_controller.borrow_mut().shutdown();
            return;
        };

        let scale = ui.window().scale_factor();

        let sync = match playback_sync.lock() {
            Ok(sync) => sync.clone(),
            Err(error) => {
                eprintln!("[Video] Playback sync mutex poisoned: {error}");
                return;
            }
        };

        video_controller.borrow_mut().sync(
            native_window_handle(&ui),
            ui.get_video_visible(),
            ui.get_current_media_url().as_str(),
            sync.request_id.as_deref(),
            &sync.playback,
            sync.control.as_ref(),
            (ui.get_video_preview_x() * scale).round() as i32,
            (ui.get_video_preview_y() * scale).round() as i32,
            (ui.get_video_preview_width() * scale).round() as i32,
            (ui.get_video_preview_height() * scale).round() as i32,
        );
    });

    video_timer
}
