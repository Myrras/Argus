//! The desktop shell: a thin Tauri 2 IPC adapter over `argus-core`.
//!
//! This crate owns no domain logic — it (de)serializes command arguments, holds
//! the live application state, and forwards work to `argus-core`.
//!
//! ## Logging & secrets
//! Logging is configured via `tauri-plugin-log` (a global `log` logger at
//! `INFO`); `tracing` events fall through to it (the `tracing/log` feature).
//! Never log secrets or untrusted output above `DEBUG`.

#![forbid(unsafe_code)]

mod commands;
mod error;
mod state;

use tauri_plugin_log::{Target, TargetKind};

use crate::state::AppState;

/// Build and run the Tauri application.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Persist window size/position across runs.
        .plugin(tauri_plugin_window_state::Builder::new().build())
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
        .manage(AppState::new())
        .invoke_handler(tauri::generate_handler![
            commands::greet::greet,
            commands::notes::note_add,
            commands::notes::note_list,
            commands::notes::note_remove,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
