mod app_dirs;
mod commands;
mod config;
mod models;
mod pipeline;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .invoke_handler(tauri::generate_handler![
            commands::pick_file,
            commands::run_pipeline,
            commands::list_models,
            commands::save_subtitle,
            commands::translate_subtitles,
            commands::list_languages,
            commands::get_settings,
            commands::set_settings,
            commands::set_api_key,
            commands::has_api_key,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
