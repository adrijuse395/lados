//! Lados: an offline password manager.
//!
//! Module map:
//! - Core, independent of Tauri: `models`, `crypto`, `storage`, `merge`, `generator`,
//!   `csv_io`, `keepass_io`, `settings`, `session`.
//! - Application, built on Tauri: `state`, `clipboard`, `background`, `tray`.
//! - The bridge to the interface: `commands` (the only module the interface talks to) and
//!   `views` (the shapes of the data it receives).

mod background;
mod clipboard;
mod commands;
mod crypto;
mod csv_io;
mod error;
mod generator;
mod keepass_io;
mod merge;
mod models;
mod session;
mod settings;
mod state;
mod storage;
mod tray;
mod views;

use std::path::PathBuf;

use tauri::{Manager, RunEvent, WindowEvent};

use crate::state::AppState;

/// A `.lados` file passed on the command line, which is how the system opens a double-clicked vault.
fn vault_from_arguments() -> Option<PathBuf> {
    std::env::args_os().skip(1).map(PathBuf::from).find(|path| {
        path.extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case(storage::FILE_EXTENSION))
            && path.is_file()
    })
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let config_dir = app.path().app_config_dir()?;
            app.manage(AppState::new(config_dir, vault_from_arguments()));
            tray::create(app.handle())?;
            background::start(app.handle().clone());
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let state = window.state::<AppState>();
                let to_tray = state.settings().preferences.close_to_tray;
                if to_tray && !state.is_quitting() {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_status,
            commands::update_preferences,
            commands::forget_recent_vault,
            commands::touch,
            commands::quit_app,
            commands::select_vault,
            commands::close_vault,
            commands::create_vault,
            commands::unlock_vault,
            commands::lock_vault,
            commands::generate_key_file,
            commands::change_master_key,
            commands::get_vault_view,
            commands::rename_vault,
            commands::pick_open_path,
            commands::pick_save_path,
            commands::search_entries,
            commands::get_entry,
            commands::get_entry_for_edit,
            commands::create_entry,
            commands::update_entry,
            commands::duplicate_entry,
            commands::move_entry,
            commands::delete_entry,
            commands::reveal_field,
            commands::copy_field,
            commands::get_entry_history,
            commands::restore_revision,
            commands::copy_revision_password,
            commands::open_entry_url,
            commands::create_folder,
            commands::update_folder,
            commands::move_folder,
            commands::delete_folder,
            commands::add_custom_icon,
            commands::generate_password,
            commands::copy_secret_text,
            commands::import_csv,
            commands::import_kdbx,
            commands::export_csv,
            commands::export_kdbx,
            commands::merge_vault_file,
        ])
        .build(tauri::generate_context!())
        .expect("Lados could not start")
        .run(|app, event| {
            if let RunEvent::Exit = event {
                commands::cleanup(app);
            }
        });
}
