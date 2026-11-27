#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use slint::ComponentHandle;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

#[macro_use]
mod logging;
mod backend;
mod config;
mod desktop_updater;
mod launcher;
mod models;
mod player_controls;
mod ui_bridge;
mod video_preview;

use models::PlaybackSyncState;
use video_preview::VideoPreviewController;

slint::include_modules!();

fn main() {
    if let Err(error) = run() {
        eprintln!("[Desktop] Fatal error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    // Acquire the single-instance lock BEFORE starting the updater.
    let _single_instance = match launcher::acquire_single_instance() {
        Ok(handle) => handle,

        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            launcher::show_error_dialog(
                "CypherSR Desktop is already running.\n\n\
                 Check your taskbar or system tray for the existing window.",
            );

            return Ok(());
        }

        Err(error) => {
            launcher::show_error_dialog(&format!(
                "Could not enforce single-instance mode:\n\n{error}"
            ));

            return Err(error.into());
        }
    };

    // Only one desktop instance may reach this point.
    let log_path = match logging::initialize() {
        Ok(path) => path,
        Err(error) => {
            launcher::show_error_dialog(&format!(
                "Could not initialize the desktop debug log:\n\n{error}"
            ));

            return Err(error.into());
        }
    };
    println!("[Logging] Writing desktop logs to {}", log_path.display());

    if !cfg!(debug_assertions) && desktop_updater::check_and_apply_update() {
        // PowerShell is waiting for this process to exit before replacing the EXE.
        return Ok(());
    }

    if let Err(error) = launcher::register_bundled_mpv_if_needed() {
        launcher::show_error_dialog(&format!(
            "Could not register the bundled MPV installation:\n\n{error}"
        ));

        return Err(error.into());
    }

    // Only the instance holding the mutex can reach this point.
    // Start the updater after the single-instance check.
    let backend_patcher = if cfg!(debug_assertions) {
        println!("[Launcher] Debug build; start the backend separately.");
        None
    } else {
        match launcher::launch_backend_patcher() {
            Ok(process) => Some(process),
            Err(error) => {
                launcher::show_error_dialog(&format!(
                    "Could not start CypherSR updater:\n\n{error}"
                ));

                return Err(error.into());
            }
        }
    };

    let config = Rc::new(RefCell::new(config::AppConfig::load_or_create()?));
    let api_base_url = config.borrow().api_base_url();
    let volume = config.borrow().playback.volume;

    println!("[Config] Backend address: {api_base_url}");

    let ui = MainWindow::new()?;
    let preferred_volume = Arc::new(Mutex::new(volume));
    ui.set_playback_volume(volume as f32);

    let video_controller = Rc::new(RefCell::new(VideoPreviewController::new(
        config.borrow().playback.mpv_path.clone(),
        api_base_url.clone(),
    )));

    let mut initial_playback = PlaybackSyncState::default();
    initial_playback.playback.volume = volume;
    let playback_sync = Arc::new(Mutex::new(initial_playback));

    let control_sender = player_controls::start_player_control_worker(api_base_url.clone());

    ui_bridge::setup_window_controls(&ui, video_controller.clone());
    ui_bridge::setup_overlay_actions(&ui, api_base_url.clone());
    ui_bridge::setup_queue_actions(&ui, api_base_url.clone());
    player_controls::setup_playback_controls(
        &ui,
        api_base_url.clone(),
        control_sender.clone(),
        preferred_volume.clone(),
        playback_sync.clone(),
        config,
    );
    player_controls::send_player_control(&control_sender, "volume", Some(volume));

    ui_bridge::start_backend_polling(
        &ui,
        playback_sync.clone(),
        api_base_url.clone(),
        preferred_volume,
    );
    ui_bridge::start_twitch_status_polling(&ui, api_base_url);

    let video_timer = ui_bridge::setup_video_preview(&ui, video_controller.clone(), playback_sync);

    let run_result = ui.run();

    // Stop GUI-owned resources before the Job Object is dropped.
    video_timer.stop();
    video_controller.borrow_mut().shutdown();
    drop(backend_patcher);

    run_result?;
    Ok(())
}
