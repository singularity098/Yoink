//! Making a desktop-entries folder outside `~/.local/share` visible to KDE
//! Plasma. Desktops look for apps in `applications/` below each folder of
//! `XDG_DATA_DIRS`; Plasma sources `~/.config/plasma-workspace/env/*.sh`
//! at login, before the session starts, so a script there that puts the
//! folder holding the desktop-entries folder in front of `XDG_DATA_DIRS`
//! makes every app of the session see it.
//!
//! The settings can be synced between computers, the script is not: what
//! state it is in is always read from the script itself, never remembered.

use crate::config::{tilde, Dirs};
use std::path::{Path, PathBuf};

/// The first line of a script Yoink wrote (it rewrites those, and leaves
/// others alone).
const MARK: &str = "# Written by Yoink";

/// Where the script goes: `$XDG_CONFIG_HOME/plasma-workspace/env/yoink.sh`.
pub fn script_path() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| home().map(|h| h.join(".config")))?;
    Some(config.join("plasma-workspace/env/yoink.sh"))
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").filter(|h| !h.is_empty()).map(PathBuf::from)
}

/// The folders desktops look in without being told: `XDG_DATA_HOME`
/// (`~/.local/share`) and the system's defaults.
fn searched_by_default() -> Vec<PathBuf> {
    let mut v = vec![PathBuf::from("/usr/local/share"), PathBuf::from("/usr/share")];
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| home().map(|h| h.join(".local/share")));
    v.extend(data_home);
    v
}

/// The folder to add to `XDG_DATA_DIRS` for the desktop-entries folder
/// `desktop`: the one it is the `applications` folder of.
pub fn data_dir_for(desktop: &Path) -> Result<PathBuf, String> {
    let desktop = strip_slash(desktop);
    if !desktop.is_absolute() {
        return Err("The desktop entries folder must be a full path.".into());
    }
    if desktop.to_string_lossy().contains(':') {
        return Err("The desktop entries folder has a ':' in its path, which XDG_DATA_DIRS cannot hold.".into());
    }
    match (desktop.file_name(), desktop.parent()) {
        (Some(name), Some(parent)) if name == "applications" => Ok(parent.to_path_buf()),
        _ => Err(format!(
            "Desktops only look in folders named applications: set the desktop entries folder to one, like {}/applications.",
            tilde(&desktop)
        )),
    }
}

fn strip_slash(p: &Path) -> PathBuf {
    p.components().collect()
}

/// What the script for `data` says.
pub fn script_text(data: &Path) -> String {
    let d = data.display().to_string();
    let escaped = d
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('`', "\\`")
        .replace('$', "\\$");
    format!(
        "{MARK} (Options → Folders), for this computer only: puts {d}\n\
         # in front of the folders desktops look in for apps (its applications/\n\
         # folder holds the desktop entries). Plasma reads it at login.\n\
         export XDG_DATA_DIRS=\"{escaped}:${{XDG_DATA_DIRS:-/usr/local/share:/usr/share}}\"\n"
    )
}

/// Whether `text` (a shell script) puts `data` into `XDG_DATA_DIRS`: an
/// uncommented line naming the variable and the folder, written out, or
/// with `$HOME`, `${HOME}` or `~` for the home folder.
fn adds(text: &str, data: &Path, home: Option<&Path>) -> bool {
    let data = strip_slash(data);
    let mut spellings = vec![data.display().to_string()];
    if let Some(rest) = home.and_then(|h| data.strip_prefix(h).ok()) {
        let rest = rest.display();
        spellings.extend([format!("$HOME/{rest}"), format!("${{HOME}}/{rest}"), format!("~/{rest}")]);
    }
    text.lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#') && l.contains("XDG_DATA_DIRS"))
        .any(|l| {
            spellings.iter().any(|s| {
                l.match_indices(s.as_str()).any(|(i, _)| {
                    // The folder must end there (a trailing slash allowed):
                    // `/a/b` is not `/a/bc` nor `/a/b/c`.
                    let rest = &l[i + s.len()..];
                    let rest = rest.strip_prefix('/').unwrap_or(rest);
                    rest.is_empty() || rest.starts_with([':', '"', '\'', ' ', ';'])
                })
            })
        })
}

/// Whether this session's `XDG_DATA_DIRS` holds `data` already.
fn active(data: &Path) -> bool {
    let data = strip_slash(data);
    std::env::var("XDG_DATA_DIRS")
        .unwrap_or_default()
        .split(':')
        .any(|d| !d.is_empty() && strip_slash(Path::new(d)) == data)
}

/// Where things stand, read from the disk and this session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// The folder the desktop entries go in cannot be added, and why.
    Unusable(String),
    /// Desktops already look there (`~/.local/share/applications`).
    NotNeeded(PathBuf),
    /// No script; this folder would be added.
    Missing(PathBuf),
    /// The script adds the folder (`ours`: Yoink wrote it). `active`: this
    /// session has it already (else it takes effect at the next login).
    Current { script: PathBuf, data: PathBuf, ours: bool, active: bool },
    /// A script is there but does not add the folder (`data`, when there is
    /// one to add).
    Other { script: PathBuf, data: Option<PathBuf>, ours: bool },
}

impl State {
    /// The script is on the disk.
    pub fn present(&self) -> bool {
        matches!(self, State::Current { .. } | State::Other { .. })
    }

    /// One line for Options.
    pub fn describe(&self) -> String {
        match self {
            State::Unusable(why) => why.clone(),
            State::NotNeeded(_) => "Not needed: desktops already look in this desktop entries folder.".into(),
            State::Missing(data) => format!(
                "Adds {} to where KDE Plasma looks for apps, from the next login. Written on this computer only, not synced.",
                tilde(data)
            ),
            State::Current { script, active: true, .. } => {
                format!("Plasma looks in this folder ({}).", tilde(script))
            }
            State::Current { script, active: false, .. } => {
                format!("Written to {}: takes effect when you next log in.", tilde(script))
            }
            State::Other { script, data: Some(data), .. } => format!(
                "{} adds another folder. Turn this off and on to point it at {}.",
                tilde(script),
                tilde(data)
            ),
            State::Other { script, data: None, .. } => {
                format!("{} is there, but this desktop entries folder needs none.", tilde(script))
            }
        }
    }
}

/// The state for the desktop-entries folder in `dirs`.
pub fn state(dirs: &Dirs) -> State {
    // Without a script: what this folder needs.
    let bare = match dirs.path("desktop") {
        Err(_) => State::Unusable("Set the desktop entries folder first.".into()),
        Ok(desktop) => match data_dir_for(&desktop) {
            Err(why) => State::Unusable(why),
            Ok(data) if searched_by_default().iter().any(|d| strip_slash(d) == data) => {
                State::NotNeeded(desktop)
            }
            Ok(data) => State::Missing(data),
        },
    };
    let Some(script) = script_path() else { return bare };
    let Ok(text) = std::fs::read_to_string(&script) else { return bare };
    let ours = text.starts_with(MARK);
    match bare {
        State::Missing(data) if adds(&text, &data, home().as_deref()) => {
            let active = active(&data);
            State::Current { script, data, ours, active }
        }
        State::Missing(data) => State::Other { script, data: Some(data), ours },
        _ => State::Other { script, data: None, ours },
    }
}

/// Writes the script for the desktop-entries folder in `dirs` (replacing
/// one that is there).
pub fn install(dirs: &Dirs) -> anyhow::Result<PathBuf> {
    use anyhow::Context;
    let data = match state(dirs) {
        State::Missing(data) | State::Current { data, .. } | State::Other { data: Some(data), .. } => data,
        State::NotNeeded(_) | State::Other { data: None, .. } => {
            anyhow::bail!("desktops already look in this desktop entries folder")
        }
        State::Unusable(why) => anyhow::bail!("{why}"),
    };
    let script = script_path().context("no home folder")?;
    crate::config::write_atomic(&script, script_text(&data).as_bytes())?;
    Ok(script)
}

/// Deletes the script, whoever wrote it (it is Yoink's by name).
pub fn remove() -> anyhow::Result<()> {
    if let Some(script) = script_path() {
        match std::fs::remove_file(&script) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
    }
    Ok(())
}

/// A script Yoink wrote that adds another folder than the desktop-entries
/// folder now set (it was changed here, or in the synced settings) is
/// rewritten for it. Returns the script when it was.
pub fn follow(dirs: &Dirs) -> anyhow::Result<Option<PathBuf>> {
    match state(dirs) {
        State::Other { ours: true, data: Some(_), .. } => install(dirs).map(Some),
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_dir_is_the_parent_of_applications() {
        assert_eq!(
            data_dir_for(Path::new("/h/Sync/Yoink/applications/")).unwrap(),
            PathBuf::from("/h/Sync/Yoink")
        );
        assert!(data_dir_for(Path::new("/h/Sync/Yoink")).is_err());
        assert!(data_dir_for(Path::new("/h/a:b/applications")).is_err());
        assert!(data_dir_for(Path::new("rel/applications")).is_err());
    }

    #[test]
    fn script_keeps_the_existing_folders() {
        let t = script_text(Path::new("/h/My \"Sync\"/Yoink"));
        assert!(t.starts_with(MARK));
        assert!(
            t.contains("export XDG_DATA_DIRS=\"/h/My \\\"Sync\\\"/Yoink:${XDG_DATA_DIRS:-/usr/local/share:/usr/share}\"\n"),
            "{t}"
        );
        assert!(adds(&script_text(Path::new("/h/S/Y")), Path::new("/h/S/Y"), None));
    }

    #[test]
    fn recognises_a_hand_written_script() {
        let home = Path::new("/home/m");
        let hand = "export XDG_DATA_DIRS=\"$HOME/Sync/Config/Yoink:${XDG_DATA_DIRS:-/usr/local/share:/usr/share}\"\n";
        assert!(adds(hand, Path::new("/home/m/Sync/Config/Yoink"), Some(home)));
        assert!(adds(hand, Path::new("/home/m/Sync/Config/Yoink/"), Some(home)));
        assert!(!adds(hand, Path::new("/home/m/Sync/Config"), Some(home)));
        assert!(!adds(hand, Path::new("/home/m/Sync/Config/Yo"), Some(home)));
        assert!(!adds(&format!("# {hand}"), Path::new("/home/m/Sync/Config/Yoink"), Some(home)));
    }
}
