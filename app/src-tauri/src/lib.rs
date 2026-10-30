//! The Voxear Launcher window. Every command is a thin layer over `voxear_launcher`, the same core `voxup`
//! uses.

mod commands;
mod dto;

use std::sync::Arc;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let launcher = commands::Launcher::new().expect("the launcher's folder can be found");
    with_commands(tauri::Builder::default(), launcher)
        // Launching again brings the open window forward instead of opening a second one.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                _ = window.unminimize();
                _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .run(tauri::generate_context!())
        .expect("the Voxear Launcher window failed to start");
}

/// The launcher's state and commands, on any runtime; the tests use Tauri's mock one.
fn with_commands<R: tauri::Runtime>(builder: tauri::Builder<R>, launcher: commands::Launcher) -> tauri::Builder<R> {
    builder.manage(Arc::new(launcher)).invoke_handler(tauri::generate_handler![
        commands::app_info,
        commands::list_installs,
        commands::list_releases,
        commands::install,
        commands::cancel_install,
        commands::install_file,
        commands::locate,
        commands::projects_using,
        commands::uninstall,
        commands::install_size,
        commands::list_projects,
        commands::add_project,
        commands::remove_project,
        commands::set_favourite,
        commands::open_project,
        commands::describe_new,
        commands::check_new_project,
        commands::create_project,
        commands::get_settings,
        commands::save_settings,
        commands::upgrade_choices,
        commands::release_notes,
        commands::upgrade_project,
        commands::list_backups,
        commands::restore_backup,
        commands::delete_backup,
    ])
}

#[cfg(test)]
mod tests;
