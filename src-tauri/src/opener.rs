//! The core's [`Opener`] for the desktop (Iris's `opener.rs`): hands a
//! file to the user's editor through tauri-plugin-opener's `open_path`
//! (macOS `open [-a app]`, Windows `ShellExecute` / the given executable)
//! and shows a file in the file manager through `reveal_item_in_dir`
//! (Finder, Explorer, or the desktop's file manager over D-Bus).
//!
//! Called from Rust only: the capability grants the webview no
//! `opener:allow-open-path` or `allow-reveal-item-in-dir`, so it can open
//! nothing but http(s) links itself. Which file and which app is the
//! core's call ([`argus_core::open`]).

use std::path::Path;

use argus_core::{CoreError, CoreResult, Opener};

pub struct DesktopOpener;

impl Opener for DesktopOpener {
    fn open(&self, path: &Path, app: Option<&str>) -> CoreResult<()> {
        tauri_plugin_opener::open_path(path, app).map_err(|e| {
            // The OS error names paths at most; still, keep the message
            // Argus-owned and put the detail in the log.
            tracing::warn!("could not open a file in its editor: {e}");
            CoreError::Io(std::io::Error::other(match app {
                Some(app) => format!("could not open the file with {app}"),
                None => "could not open the file: no application is set for its type".into(),
            }))
        })
    }

    fn reveal(&self, path: &Path) -> CoreResult<()> {
        tauri_plugin_opener::reveal_item_in_dir(path).map_err(|e| {
            tracing::warn!("could not show a file in the file manager: {e}");
            CoreError::Io(std::io::Error::other(
                "could not show the file in the file manager",
            ))
        })
    }
}
