//! The GitHub token: in the desktop's keyring (Secret Service), or, where
//! there is none, in `~/.config/yoink/token` readable only by you.

use std::path::PathBuf;

use crate::config::config_dir;

const SERVICE: &str = "yoink";
const USER: &str = "github-token";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Store {
    Keyring,
    File,
}

impl Store {
    pub fn label(self) -> &'static str {
        match self {
            Store::Keyring => "the keyring",
            Store::File => "a private file (no keyring found)",
        }
    }
}

fn file() -> PathBuf {
    config_dir().join("token")
}

fn entry() -> Option<keyring::Entry> {
    keyring::Entry::new(SERVICE, USER).ok()
}

/// The token and where it was found.
pub fn load() -> Option<(String, Store)> {
    if let Some(e) = entry() {
        match e.get_password() {
            Ok(t) if !t.trim().is_empty() => return Some((t.trim().to_string(), Store::Keyring)),
            Ok(_) | Err(keyring::Error::NoEntry) => {}
            Err(err) => log::info!("keyring not available: {err}"),
        }
    }
    let t = std::fs::read_to_string(file()).ok()?;
    let t = t.trim();
    (!t.is_empty()).then(|| (t.to_string(), Store::File))
}

/// Saves the token: in the keyring if there is one, else in the file.
pub fn save(token: &str) -> anyhow::Result<Store> {
    let token = token.trim();
    anyhow::ensure!(!token.is_empty(), "the token is empty");
    if let Some(e) = entry() {
        match e.set_password(token) {
            Ok(()) => {
                // Do not leave an older copy in the file behind.
                let _ = std::fs::remove_file(file());
                return Ok(Store::Keyring);
            }
            Err(err) => log::warn!("could not use the keyring ({err}); saving to a file"),
        }
    }
    write_private(token)?;
    Ok(Store::File)
}

pub fn clear() {
    if let Some(e) = entry() {
        let _ = e.delete_credential();
    }
    let _ = std::fs::remove_file(file());
}

fn write_private(token: &str) -> anyhow::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let path = file();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)?;
    f.write_all(token.as_bytes())?;
    f.sync_all()?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}
