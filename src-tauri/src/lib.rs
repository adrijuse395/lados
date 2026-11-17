//! Lados: an offline password manager.
//!
//! Module map:
//! - Core, independent of Tauri: `models`, `crypto`, `storage`, `merge`, `generator`,
//!   `csv_io`, `keepass_io`, `settings`, `session`.
//! - Application, built on Tauri: `state`, `clipboard`, `background`, `tray`.
//! - The bridge to the interface: `commands` (the only module the interface talks to) and
//!   `views` (the shapes of the data it receives).

#[cfg(desktop)]
mod autofill_http;
mod background;
mod browsers;
mod clipboard;
mod commands;
mod crypto;
mod csv_io;
mod error;
mod generator;
mod keepass_io;
mod merge;
mod models;
mod recovery;
mod session;
mod settings;
mod state;
mod storage;
#[cfg(feature = "sync")]
pub mod sync;
#[cfg(desktop)]
mod tray;
mod views;

use std::path::PathBuf;

use tauri::{Manager, RunEvent};
#[cfg(desktop)]
use tauri::WindowEvent;

use crate::state::AppState;

/// A `.lados` file passed on the command line, which is how the system opens a double-clicked vault.
fn vault_from_arguments() -> Option<PathBuf> {
    std::env::args_os().skip(1).map(PathBuf::from).find(|path| {
        path.extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case(storage::FILE_EXTENSION))
            && path.is_file()
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // rustls needs a process-wide crypto provider. We install `ring` (enabled on our `rustls`
    // dependency); this keeps reqwest off its default `rustls` feature, which would pull the very
    // slow aws-lc-sys C build.
    #[cfg(feature = "sync")]
    let _ = rustls::crypto::ring::default_provider().install_default();

    let builder = tauri::Builder::default();

    // Single-instance runs first: a second launch (for example from the browser
    // extension's `lados://` link) is forwarded to the running window instead of
    // opening a duplicate. The deep-link plugin receives that scheme.
    #[cfg(desktop)]
    let builder = builder
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ));

    builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            #[cfg(desktop)]
            {
                use tauri_plugin_deep_link::DeepLinkExt;
                // In development the `lados://` scheme is not installed system-wide
                // yet; register it so the browser extension's link works under `tauri dev`.
                let _ = app.deep_link().register_all();
                app.handle()
                    .plugin(tauri_plugin_updater::Builder::new().build())?;
                app.handle().plugin(tauri_plugin_process::init())?;
            }
            let config_dir = app.path().app_config_dir()?;
            app.manage(AppState::new(config_dir, vault_from_arguments()));
            #[cfg(desktop)]
            tray::create(app.handle())?;
            background::start(app.handle().clone());
            #[cfg(desktop)]
            autofill_http::start(app.handle().clone());
            Ok(())
        })
        .on_window_event(|window, event| {
            #[cfg(desktop)]
            if let WindowEvent::CloseRequested { api, .. } = event {
                let state = window.state::<AppState>();
                let to_tray = state.settings().preferences.close_to_tray;
                if to_tray && !state.is_quitting() {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
            #[cfg(mobile)]
            let _ = (window, event);
        })
        .invoke_handler(tauri::generate_handler![
            browsers::detected_browsers,
            browsers::open_in_browser,
            commands::get_status,
            commands::update_preferences,
            commands::forget_recent_vault,
            commands::remove_listed_vault,
            commands::touch,
            commands::quit_app,
            commands::sync::sync_status,
            commands::sync::sync_sign_in,
            commands::sync::sync_register,
            commands::sync::sync_sign_out,
            commands::sync::sync_enable,
            commands::sync::sync_enable_file,
            commands::sync::rekey_vault_file,
            commands::sync::sync_refresh,
            commands::billing::billing_checkout,
            commands::billing::billing_cancel,
            commands::billing::billing_resume,
            commands::account::change_master_password,
            commands::account::resync_labels,
            commands::sync::sync_disable,
            commands::sync::sync_disable_file,
            commands::sync::sync_disable_vault,
            commands::sync::sync_now,
            commands::account::sync_delete_account,
            commands::sync::sync_cloud_vaults,
            commands::sync::sync_delete_vault,
            commands::sync::sync_sign_out_everywhere,
            commands::sync::sync_download,
            commands::vault::select_vault,
            commands::vault::close_vault,
            commands::vault::create_vault,
            commands::vault::unlock_vault,
            commands::vault::lock_vault,
            commands::vault::generate_key_file,
            commands::vault::change_master_key,
            commands::recovery::create_recovery_key,
            commands::recovery::recover_vault,
            commands::recovery::check_recovery_key,
            commands::vault::get_vault_view,
            commands::vault::rename_vault,
            commands::vault::pick_open_path,
            commands::vault::pick_save_path,
            commands::vault::pick_folder,
            commands::is_mobile_shell,
            commands::suggested_vault_path,
            commands::vaults_folder,
            commands::list_device_vaults,
            commands::entries::search_entries,
            commands::entries::find_entries_for_url,
            commands::entries::get_autofill_payload,
            commands::entries::get_entry,
            commands::entries::get_entry_for_edit,
            commands::entries::create_entry,
            commands::entries::update_entry,
            commands::entries::duplicate_entry,
            commands::entries::move_entry,
            commands::entries::delete_entry,
            commands::entries::reveal_field,
            commands::entries::copy_field,
            commands::entries::get_entry_history,
            commands::entries::restore_revision,
            commands::entries::copy_revision_password,
            commands::entries::open_entry_url,
            commands::entries::open_external_url,
            commands::entries::get_autostart,
            commands::entries::set_autostart,
            commands::entries::create_folder,
            commands::entries::update_folder,
            commands::entries::move_folder,
            commands::entries::delete_folder,
            commands::entries::add_custom_icon,
            commands::entries::generate_password,
            commands::entries::copy_secret_text,
            commands::transfer::import_csv,
            commands::transfer::import_kdbx,
            commands::transfer::export_csv,
            commands::transfer::export_kdbx,
            commands::transfer::merge_vault_file,
        ])
        .build(tauri::generate_context!())
        .expect("Lados could not start")
        .run(|app, event| {
            if let RunEvent::Exit = event {
                commands::cleanup(app);
            }
        });
}
