//! Argus' own `scripts/` folder (next to `settings.json`): where a script
//! written with the AI's help is saved. Saving writes a file and nothing
//! else; the Scripts tab then adds the entry that points at it.

use std::path::{Path, PathBuf};

use crate::error::{CoreError, CoreResult};
use crate::Core;

/// The folder's name under Argus' data folder.
pub(crate) const SCRIPTS_DIR: &str = "scripts";

/// The longest file name (without `.sh`) a save makes.
const MAX_SLUG: usize = 64;

/// The largest script Argus saves or reads back.
pub(crate) const MAX_SCRIPT: usize = 256 * 1024;

/// `name` as a file name: lower-case ASCII letters and digits, the rest
/// one `-` each run, no `-` at the ends, at most [`MAX_SLUG`] characters,
/// then `.sh` (a `.sh` the name ends with is not doubled). An error for a
/// name with a folder in it (`/`, `\`, `..`) or nothing left.
pub(crate) fn file_name(name: &str) -> CoreResult<String> {
    let name = name.trim();
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(CoreError::InvalidInput(
            "the file name can't contain a folder".into(),
        ));
    }
    let stem = name
        .strip_suffix(".sh")
        .or_else(|| name.strip_suffix(".SH"))
        .unwrap_or(name);
    let mut slug = String::new();
    for c in stem.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug: String = slug.trim_matches('-').chars().take(MAX_SLUG).collect();
    let slug = slug.trim_end_matches('-');
    if slug.is_empty() {
        return Err(CoreError::InvalidInput(
            "give the script a name with letters or digits".into(),
        ));
    }
    Ok(format!("{slug}.sh"))
}

/// Write `content` to `dir/<file_name(name)>` with mode 0755 (Unix):
/// atomically (a temp file renamed over), never over an existing file
/// unless `overwrite` (a [`CoreError::Conflict`] otherwise). The path.
pub(crate) fn save_in(
    dir: &Path,
    name: &str,
    content: &str,
    overwrite: bool,
) -> CoreResult<PathBuf> {
    let file = file_name(name)?;
    if content.trim().is_empty() {
        return Err(CoreError::InvalidInput("the script is empty".into()));
    }
    if content.len() > MAX_SCRIPT {
        return Err(CoreError::InvalidInput("the script is too large".into()));
    }
    std::fs::create_dir_all(dir)?;
    let path = dir.join(&file);
    if !overwrite && path.symlink_metadata().is_ok() {
        return Err(CoreError::Conflict(format!(
            "A script named {file} already exists."
        )));
    }
    let mut text = content.replace("\r\n", "\n");
    if !text.ends_with('\n') {
        text.push('\n');
    }
    let tmp = dir.join(format!(".{file}.tmp"));
    std::fs::write(&tmp, text)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
    }
    std::fs::rename(&tmp, &path)?;
    Ok(path)
}

/// `command` resolved to a file inside `dir` (a script Argus saved), else
/// `None`: nothing outside the folder is read back.
pub(crate) fn inside(dir: &Path, command: &str) -> Option<PathBuf> {
    let dir = dir.canonicalize().ok()?;
    let file = Path::new(command.trim()).canonicalize().ok()?;
    (file.starts_with(&dir) && file.is_file()).then_some(file)
}

impl Core {
    /// Where scripts written with the AI's help are saved: `scripts/` next
    /// to `settings.json`. `None` for an in-memory core (tests set one).
    pub fn submit_scripts_dir(&self) -> Option<PathBuf> {
        self.scripts_dir
            .lock()
            .expect("scripts dir lock poisoned")
            .clone()
    }

    /// Point [`submit_scripts_dir`](Self::submit_scripts_dir) elsewhere
    /// (tests); `None` turns saving off.
    pub fn set_submit_scripts_dir(&self, dir: Option<PathBuf>) {
        *self.scripts_dir.lock().expect("scripts dir lock poisoned") = dir;
    }

    /// Save a script in [`submit_scripts_dir`](Self::submit_scripts_dir)
    /// as `<slug of name>.sh`, executable; a [`CoreError::Conflict`] when
    /// one by that name is there and `overwrite` is not set. Writes the
    /// file only: the settings entry is the caller's, and nothing runs.
    pub fn save_submit_script(
        &self,
        name: &str,
        content: &str,
        overwrite: bool,
    ) -> CoreResult<PathBuf> {
        let dir = self
            .submit_scripts_dir()
            .ok_or_else(|| CoreError::InvalidInput("Argus has no folder for scripts".into()))?;
        save_in(&dir, name, content, overwrite)
    }

    /// The text of the script `script_id` (to edit it with the AI's help):
    /// only one Argus saved, inside its scripts folder.
    pub fn read_submit_script(&self, script_id: &str) -> CoreResult<String> {
        let script = self
            .settings()
            .submit_scripts
            .into_iter()
            .find(|s| s.id == script_id.trim())
            .ok_or(CoreError::NotFound)?;
        let dir = self.submit_scripts_dir().ok_or(CoreError::NotFound)?;
        let file = inside(&dir, &script.command).ok_or_else(|| {
            CoreError::InvalidInput("only a script Argus saved can be read back".into())
        })?;
        if std::fs::metadata(&file)?.len() > MAX_SCRIPT as u64 {
            return Err(CoreError::InvalidInput("the script is too large".into()));
        }
        let bytes = std::fs::read(&file)?;
        String::from_utf8(bytes)
            .map_err(|_| CoreError::InvalidInput("the script is not text".into()))
    }

    /// Show the scripts folder in Finder / Explorer (made when missing).
    pub fn reveal_submit_scripts_dir(&self) -> CoreResult<()> {
        let dir = self
            .submit_scripts_dir()
            .ok_or_else(|| CoreError::InvalidInput("Argus has no folder for scripts".into()))?;
        std::fs::create_dir_all(&dir)?;
        self.opener().reveal(&dir)
    }
}
