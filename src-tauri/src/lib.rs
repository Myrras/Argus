//! The desktop shell: a thin Tauri 2 IPC adapter over `argus-core`.
//!
//! This crate owns no domain logic — it (de)serializes command arguments, holds
//! the live application state, and forwards work to `argus-core`. Refresh
//! results reach the webview as events (see [`events`]).
//!
//! ## Logging & secrets
//! Logging is configured via `tauri-plugin-log` (a global `log` logger at
//! `INFO`); `tracing` events fall through to it (the `tracing/log` feature).
//! Never log secrets or untrusted output above `DEBUG`.

#![forbid(unsafe_code)]

mod commands;
mod error;
mod events;
mod state;

#[cfg(target_os = "macos")]
use tauri::Emitter;
use tauri::Manager;
use tauri_plugin_log::{Target, TargetKind};

use crate::state::AppState;

/// Build and run the Tauri application.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Persist window size/position across runs. Decorations excluded (as
        // in Iris): its on_window_ready restore runs after the platform
        // config is applied and, with default StateFlags, would reassert the
        // native title bar (a persisted `"decorated": true`) on every launch.
        .plugin(
            tauri_plugin_window_state::Builder::new()
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::all()
                        - tauri_plugin_window_state::StateFlags::DECORATIONS,
                )
                .build(),
        )
        // Opens review links in the OS default browser.
        .plugin(tauri_plugin_opener::init())
        // Native folder picker for a repository's local clone (Settings → AI).
        .plugin(tauri_plugin_dialog::init())
        // Logging sinks: stdout (dev), the per-OS log directory (support), and
        // the webview console (in-app diagnostics).
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(tauri_plugin_log::log::LevelFilter::Info)
                .targets([
                    Target::new(TargetKind::Stdout),
                    Target::new(TargetKind::LogDir { file_name: None }),
                    Target::new(TargetKind::Webview),
                ])
                .build(),
        )
        .setup(|app| {
            // macOS: native menu bar with Settings… (⌘,) in the app menu, as in
            // Iris; the other platforms use the in-app title-bar gear instead.
            // The menu item emits a Tauri event the frontend listens for.
            #[cfg(target_os = "macos")]
            {
                use tauri::menu::{MenuBuilder, MenuItem, SubmenuBuilder};

                let settings = MenuItem::with_id(
                    app,
                    "settings",
                    "Settings\u{2026}", // "Settings…"
                    true,
                    Some("CmdOrCtrl+,"),
                )?;
                let app_menu = SubmenuBuilder::new(app, "Argus")
                    .about(None)
                    .separator()
                    .item(&settings)
                    .separator()
                    .services()
                    .separator()
                    .hide()
                    .hide_others()
                    .show_all()
                    .separator()
                    .quit()
                    .build()?;
                // Setting a custom menu replaces the default one, so Edit must
                // be re-added or Cmd+C/V/X stop working in the webview.
                let edit_menu = SubmenuBuilder::new(app, "Edit")
                    .undo()
                    .redo()
                    .separator()
                    .cut()
                    .copy()
                    .paste()
                    .separator()
                    .select_all()
                    .build()?;
                let menu = MenuBuilder::new(app)
                    .item(&app_menu)
                    .item(&edit_menu)
                    .build()?;
                app.set_menu(menu)?;
                app.on_menu_event(|app, event| {
                    if event.id() == "settings" {
                        let _ = app.emit("menu:open-settings", ());
                    }
                });
            }

            // The database and settings.json live in the app data dir, which
            // only exists once the app does — hence managing state here
            // rather than on the builder.
            let state = AppState::open(&app.path().app_data_dir()?)?;
            start_polling(app.handle().clone(), &state);
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::review::list_reviews,
            commands::review::get_review,
            commands::review::refresh_reviews,
            commands::review::last_refresh,
            commands::review::mark_reviewed,
            commands::thread::list_threads,
            commands::settings::get_settings,
            commands::settings::update_settings,
            commands::settings::set_gitlab_token,
            commands::settings::gitlab_token_present,
            commands::provider::provider_status,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Start the core's background refresh loop. It refreshes at once, then at
/// the settings' interval (a settings change restarts it with the same
/// callback); every report is remembered for `last_refresh` and emitted.
fn start_polling(app: tauri::AppHandle, state: &AppState) {
    let last = state.last_refresh.clone();
    state.core.start_polling(move |report| {
        AppState::record(&last, report);
        events::emit_report(&app, report);
    });
}
