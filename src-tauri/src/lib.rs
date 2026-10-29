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
mod opener;
mod state;

use std::path::Path;

#[cfg(target_os = "macos")]
use tauri::Emitter;
use tauri::{Manager, RunEvent};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
use tauri_plugin_log::{Target, TargetKind};

use crate::state::AppState;

/// Build and run the Tauri application.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        // First: a second launch hands over to the running Argus (whose
        // window comes to the front) and exits before anything opens the
        // database, polls, or marks the first one's AI runs interrupted.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            focus_main_window(app);
        }))
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
        // Native folder picker for a repository's local clone (Settings → Repositories),
        // and the startup error dialog.
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
                // The standard Window menu: without it ⌘M, the zoom and
                // full-screen items and ⌘W do nothing.
                let window_menu = SubmenuBuilder::new(app, "Window")
                    .minimize()
                    .maximize()
                    .fullscreen()
                    .separator()
                    .close_window()
                    .build()?;
                let menu = MenuBuilder::new(app)
                    .item(&app_menu)
                    .item(&edit_menu)
                    .item(&window_menu)
                    .build()?;
                app.set_menu(menu)?;
                app.on_menu_event(|app, event| {
                    if event.id() == "settings" {
                        let _ = app.emit("menu:open-settings", ());
                    }
                });
            }

            // Temp dirs a previous Argus left behind (it crashed or was killed
            // mid-run) hold private diffs. Safe here: single-instance means no
            // run of another Argus is live.
            let swept = argus_core::ai::sweep_stale_temp_dirs();
            if swept > 0 {
                tracing::info!("removed {swept} stale AI run dir(s)");
            }

            // The database and settings.json live in the app data dir, which
            // only exists once the app does — hence managing state here
            // rather than on the builder. A failure is shown and Argus quits:
            // an error out of setup would abort without a word.
            let opened = app
                .path()
                .app_data_dir()
                .map_err(|e| (None, e.to_string()))
                .and_then(|dir| {
                    AppState::open(&dir).map_err(|e| (Some(dir.clone()), e.to_string()))
                });
            let state = match opened {
                Ok(state) => state,
                Err((dir, error)) => {
                    let message = startup_error_message(dir.as_deref(), &error);
                    tracing::error!("{message}");
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.hide();
                    }
                    let handle = app.handle().clone();
                    // Not `blocking_show`: setup runs on the main thread.
                    app.dialog()
                        .message(message)
                        .title("Argus could not start")
                        .kind(MessageDialogKind::Error)
                        .show(move |_| handle.exit(1));
                    return Ok(());
                }
            };
            // Before the first poll, so the runs it queues are reported.
            let handle = app.handle().clone();
            state
                .core
                .on_ai_state(move |ai| events::emit_ai_state(&handle, ai));
            let handle = app.handle().clone();
            state
                .core
                .on_scripts_done(move |done| events::emit_scripts_done(&handle, done));
            let handle = app.handle().clone();
            state
                .core
                .on_script_draft(move |draft| events::emit_script_draft(&handle, draft));
            let handle = app.handle().clone();
            state.core.on_script_draft_progress(move |progress| {
                events::emit_script_draft_progress(&handle, progress)
            });
            // Open in editor: files go to the editor from Rust, not the webview.
            state
                .core
                .set_opener(std::sync::Arc::new(opener::DesktopOpener));
            // Review worktrees a previous Argus left in the clones (it quit
            // mid-run): only its own, marked ones. Off the main thread (git
            // deletes their files); a run sweeps its clone first anyway.
            let core = std::sync::Arc::clone(&state.core);
            std::thread::spawn(move || {
                let swept = core.sweep_review_worktrees();
                if swept > 0 {
                    tracing::info!("removed {swept} stale review worktree(s)");
                }
            });
            // Ask gh and glab what they are logged in to (a request per
            // host), then poll their hosts: off the main thread.
            let handle = app.handle().clone();
            let core = std::sync::Arc::clone(&state.core);
            let last = state.last_refresh.clone();
            std::thread::spawn(move || {
                core.list_accounts(false);
                start_polling(handle, &core, last);
            });
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::review::list_reviews,
            commands::review::get_review,
            commands::review::refresh_reviews,
            commands::review::refresh_review,
            commands::review::last_refresh,
            commands::thread::list_threads,
            commands::files::list_files,
            commands::files::file_contents,
            commands::files::file_contents_batch,
            commands::files::warm_file_cache,
            commands::files::cache_stats,
            commands::files::clear_cache,
            commands::files::set_file_viewed,
            commands::post::post_comments,
            commands::post::reply_to_thread,
            commands::post::approve_review,
            commands::post::resolve_thread,
            commands::post::commentable_lines,
            commands::post::commentable_lines_batch,
            commands::post::submit_review,
            commands::settings::get_settings,
            commands::settings::update_settings,
            commands::provider::list_accounts,
            commands::history::thread_history,
            commands::history::review_histories,
            commands::ai::ai_state,
            commands::ai::list_findings,
            commands::ai::list_checklist,
            commands::ai::list_verdicts,
            commands::ai::list_skipped_findings,
            commands::ai::show_finding_anyway,
            commands::ai::list_skipped_checklist,
            commands::ai::show_checklist_item_anyway,
            commands::ai::run_ai,
            commands::ai::cancel_ai,
            commands::ai::dismiss_finding,
            commands::ai::set_checklist_checked,
            commands::ai::set_verdict_override,
            commands::ai::post_findings,
            commands::ai::list_review_skills,
            commands::open::local_files,
            commands::open::open_in_editor,
            commands::open::reveal_local_file,
            commands::open::clone_status,
            commands::open::reveal_clone,
            commands::open::suggest_clone,
            commands::open::use_clone_suggestion,
            commands::open::ignore_clone_suggestion,
            commands::repos::list_repositories,
            commands::repos::reveal_repo_clone,
            commands::scripts::script_generation_status,
            commands::scripts::start_script_draft,
            commands::scripts::send_script_draft,
            commands::scripts::retry_script_draft,
            commands::scripts::cancel_script_draft,
            commands::scripts::list_script_drafts,
            commands::scripts::discard_script_draft,
            commands::scripts::link_script_draft,
            commands::scripts::save_submit_script,
            commands::scripts::read_submit_script,
            commands::scripts::submit_scripts_dir,
            commands::scripts::reveal_submit_scripts_dir,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app, event| {
        // Quitting: cancel the AI runs and kill their agents (else they run
        // on as orphans, their temp dirs with private diffs left behind),
        // then stop polling. `try_state`: the state is missing when startup
        // failed.
        if let RunEvent::Exit = event {
            if let Some(state) = app.try_state::<AppState>() {
                state.core.shutdown();
            }
        }
    });
}

/// Bring the main window to the front (a second launch of Argus).
fn focus_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// What the startup error dialog says: what failed and where Argus keeps
/// its data, so the user can look there (or move the folder aside).
/// `error` is app-owned text (`CoreError`'s), never a secret.
fn startup_error_message(data_dir: Option<&Path>, error: &str) -> String {
    match data_dir {
        Some(dir) => format!(
            "Argus could not open its data in {}: {error}\n\nArgus will quit.",
            dir.display()
        ),
        None => format!("Argus could not find its data folder: {error}\n\nArgus will quit."),
    }
}

/// Start the core's background refresh loop. It refreshes at once, then at
/// the settings' interval (a settings change restarts it with the same
/// callback); every report is remembered for `last_refresh`, emitted, and
/// handed to the AI pass, which queues the runs it calls for (it never
/// waits on one).
fn start_polling(
    app: tauri::AppHandle,
    core: &std::sync::Arc<argus_core::Core>,
    last: std::sync::Arc<std::sync::Mutex<Option<argus_core::RefreshReport>>>,
) {
    // Weak: the core owns this callback, a strong handle would be a cycle.
    let weak = std::sync::Arc::downgrade(core);
    core.start_polling(move |report| {
        AppState::record(&last, report);
        events::emit_report(&app, report);
        if let Some(core) = weak.upgrade() {
            core.on_refresh_report(report);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_startup_error_names_the_data_dir() {
        let m = startup_error_message(
            Some(Path::new(
                "/Users/fixture/Library/Application Support/com.argus.app",
            )),
            "the database is from a newer Argus; update Argus",
        );
        assert!(m.contains("/Users/fixture/Library/Application Support/com.argus.app"));
        assert!(m.contains("the database is from a newer Argus"));
        assert!(m.ends_with("Argus will quit."));
        assert!(startup_error_message(None, "no home").contains("could not find its data folder"));
    }
}
