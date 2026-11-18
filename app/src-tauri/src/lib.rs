//! The Voxear Launcher window. Every command is a thin layer over `voxear_launcher`, the same core `voxup`
//! uses.

mod cli;
mod commands;
mod dto;
mod updates;

use std::sync::Arc;

use tauri::{Manager, RunEvent};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let context = tauri::generate_context!();
    match cli::parse(std::env::args_os().skip(1)) {
        Ok(cli::Action::Window) => {}
        Ok(action) => std::process::exit(cli::run(action, context)),
        Err(error) => {
            eprintln!("error: {error}");
            std::process::exit(2);
        }
    }

    let launcher = commands::Launcher::new().expect("the launcher's folder can be found");
    let app = with_commands(tauri::Builder::default(), launcher)
        // Launching again brings the open window forward instead of opening a second one.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                _ = window.unminimize();
                _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(updater())
        .build(context)
        .expect("the Voxear Launcher window failed to start");
    app.run(|app, event| {
        // Plugins see the exit first, so the single-instance lock is gone before the next copy starts.
        if let RunEvent::Exit = event
            && let Some(program) = app.state::<commands::Relaunch>().0.lock().expect("never poisoned").take()
            && let Err(error) = std::process::Command::new(&program).spawn()
        {
            eprintln!("couldn't start {}: {error}", program.display());
        }
    });
}

/// Tauri's updater, with the launcher update key built into the core rather than into `tauri.conf.json`, so the
/// key lives beside the engine release keys and a build without one still starts.
fn updater<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R, tauri_plugin_updater::Config> {
    let builder = tauri_plugin_updater::Builder::new();
    match voxear_launcher::self_update::update_key() {
        Some(key) => builder.pubkey(key).build(),
        None => builder.build(),
    }
}

/// The launcher's state and commands, on any runtime; the tests use Tauri's mock one.
fn with_commands<R: tauri::Runtime>(builder: tauri::Builder<R>, launcher: commands::Launcher) -> tauri::Builder<R> {
    let builder = builder
        .manage(Arc::new(launcher))
        .manage(commands::PendingUpdate::default())
        .manage(commands::Relaunch::default());
    builder.invoke_handler(tauri::generate_handler![
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
        commands::add_custom_releases,
        commands::remove_custom_releases,
        commands::upgrade_choices,
        commands::release_notes,
        commands::upgrade_project,
        commands::list_backups,
        commands::restore_backup,
        commands::delete_backup,
        commands::check_launcher_update,
        commands::install_launcher_update,
        commands::restart_launcher,
        commands::install_launcher_app,
        commands::add_module,
        commands::remove_module,
        commands::whisk_status,
        commands::install_whisk,
        commands::open_whisk,
        commands::remove_whisk,
    ])
}

#[cfg(test)]
mod tests;
