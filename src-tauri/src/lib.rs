use tauri::Manager;

mod app_dirs;
mod cancel;
mod cloud_models;
mod commands;
mod config;
mod download;
mod errors;
mod http;
#[cfg(target_os = "linux")]
mod menu_style;
mod models;
mod paths;
mod pipeline;
mod release_notes;

/// Temporary audio older than this belongs to a run that no longer exists.
const STALE_AUDIO_AGE: std::time::Duration = std::time::Duration::from_secs(3600);

/// Waiting this long for a run to clean up when the user quits during it.
const QUIT_GRACE: std::time::Duration = std::time::Duration::from_secs(5);

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(target_os = "linux")]
    menu_style::unhinted_text();
    let app = tauri::Builder::default()
        // A second launch brings the first window forward instead of starting a second
        // process that would share the same files.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        // A dropped file reaches the webview as a lossy string; remember the real path so a
        // file name that is not valid UTF-8 still resolves to the right file.
        .on_window_event(|_window, event| {
            if let tauri::WindowEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }) = event {
                for path in paths {
                    paths::to_webview(path);
                    paths::allow_input(path);
                }
            }
        })
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            #[cfg(target_os = "linux")]
            menu_style::style_menu_bar();
            // Audio left behind by a crash or a forced quit.
            if let Ok(cache) = app.path().app_cache_dir() {
                pipeline::audio_extract::remove_stale_temp_dirs(&cache, STALE_AUDIO_AGE);
            }
            models::remove_legacy_files();
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::pick_file,
            commands::run_pipeline,
            commands::list_audio_tracks,
            commands::update_subtitles,
            commands::save_subtitle_as,
            commands::cancel_pipeline,
            commands::delete_api_key,
            commands::translate_subtitles,
            commands::list_languages,
            commands::get_settings,
            commands::set_settings,
            commands::set_api_key,
            commands::has_api_key,
            release_notes::release_notes,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|handle, event| {
        // Closing the window during a run: ask it to stop and wait for it to delete its
        // temporary audio and end ffmpeg, instead of exiting under it.
        if let tauri::RunEvent::ExitRequested {
            api, code: None, ..
        } = event
        {
            if cancel::is_busy() {
                api.prevent_exit();
                cancel::request();
                let handle = handle.clone();
                std::thread::spawn(move || {
                    let deadline = std::time::Instant::now() + QUIT_GRACE;
                    while cancel::is_busy() && std::time::Instant::now() < deadline {
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                    handle.exit(0);
                });
            }
        }
    });
}
