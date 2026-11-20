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

/// What a check of the token found: whether GitHub takes it, and a warning
/// when it can do more than Yoink needs.
pub struct Check {
    pub note: String,
    pub warning: Option<String>,
}

/// Asks GitHub about the token. Yoink only reads releases, Actions runs and
/// their files, so a token that can write, or see private repositories, is
/// worth less to Yoink than it would be to whoever took it.
pub fn check(token: &str) -> Check {
    let auth = crate::http::Auth {
        token: Some(token.to_string()),
        hosts: &["api.github.com"],
    };
    let resp = match crate::http::get("https://api.github.com/rate_limit", "application/vnd.github+json", &auth) {
        Ok(r) => r,
        Err(e) => {
            return Check {
                note: format!("It did not work: {}", e.to_string().lines().next().unwrap_or("")),
                warning: None,
            }
        }
    };
    // Classic tokens (and OAuth ones) list their scopes; fine-grained ones
    // do not send the header at all.
    let scopes = resp.header("x-oauth-scopes").map(str::to_string);
    let v: serde_json::Value = resp.into_json().unwrap_or_default();
    let left = v.pointer("/resources/core/remaining").and_then(|x| x.as_u64());
    let limit = v.pointer("/resources/core/limit").and_then(|x| x.as_u64());
    let note = match (left, limit) {
        (Some(l), Some(m)) if m > 60 => format!("It works: {l} of {m} requests left this hour."),
        _ => {
            return Check {
                note: "GitHub did not take it as a token (still 60 requests an hour).".into(),
                warning: None,
            }
        }
    };
    let sees_private = match scopes {
        Some(_) => None,
        None => crate::http::get_json::<Vec<serde_json::Value>>(
            "https://api.github.com/user/repos?visibility=private&per_page=1",
            &auth,
        )
        .ok()
        .map(|repos| !repos.is_empty()),
    };
    Check {
        note,
        warning: warning(scopes.as_deref(), sees_private),
    }
}

/// The warning for a token with these classic scopes (None: a fine-grained
/// token) that can or cannot see private repositories.
fn warning(scopes: Option<&str>, sees_private: Option<bool>) -> Option<String> {
    match scopes {
        Some(s) => {
            let list: Vec<&str> = s.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
            (!list.is_empty()).then(|| {
                format!(
                    "This token has more access than Yoink needs ({}). If it leaked, it could be used to change your repositories or account. For public repositories, a classic token with no scopes is enough.",
                    list.join(", ")
                )
            })
        }
        None => (sees_private == Some(true)).then(|| {
            "This token can see private repositories. If Yoink only needs public ones, a fine-grained token limited to public repositories (read-only) is safer if it leaks.".to_string()
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::warning;

    #[test]
    fn warns_about_scopes() {
        assert_eq!(warning(Some(""), None), None);
        let w = warning(Some("repo, workflow"), None).unwrap();
        assert!(w.contains("(repo, workflow)"), "{w}");
    }

    #[test]
    fn warns_about_private_access() {
        assert_eq!(warning(None, Some(false)), None);
        assert_eq!(warning(None, None), None);
        assert!(warning(None, Some(true)).unwrap().contains("private"));
    }
}
