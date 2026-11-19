//! Where apps are published. A forge finds the newest build a deployment
//! wants, lists the files a channel offers (for picking one when adding an
//! app), downloads, and finds an icon. GitHub is the only one so far; others
//! (GitLab, Codeberg, plain URLs) implement the same trait.

use std::path::Path;
use std::sync::atomic::AtomicBool;

use crate::library::{Channel, IconSource, Source};

pub mod github;

/// The build a deployment would install now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Remote {
    /// Identifies the build: a different key from the installed one is an
    /// update.
    pub key: String,
    /// The version as shown: a release tag, or a run number and commit.
    pub version: String,
    /// The file downloaded.
    pub file_name: String,
    pub size: u64,
    pub url: String,
    pub accept: String,
    /// Only downloadable with a token (Actions artifacts).
    pub needs_token: bool,
    /// Where a SHA-256 for the file was published, if anywhere.
    pub checksum: Option<ChecksumSource>,
    /// Something worth knowing ("a newer build is in progress").
    pub note: String,
    /// The release or run's page.
    pub web: String,
    pub published: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChecksumSource {
    pub url: String,
    pub accept: String,
}

/// A file a channel offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub name: String,
    pub size: u64,
}

/// What a channel offers now, for picking.
#[derive(Clone, Debug, Default)]
pub struct Offer {
    pub files: Vec<Candidate>,
    /// The release tag, or the run, the files are from.
    pub version: String,
    pub note: String,
}

#[derive(Clone, Debug, Default)]
pub struct RepoInfo {
    pub name: String,
    pub description: String,
    pub default_branch: String,
    /// The workflows' file names (Actions).
    pub workflows: Vec<String>,
}

pub struct Icon {
    pub bytes: Vec<u8>,
    pub ext: &'static str,
    pub source: IconSource,
}

pub trait Forge: Send {
    fn info(&self) -> anyhow::Result<RepoInfo>;
    fn offer(&self, channel: &Channel) -> anyhow::Result<Offer>;
    fn resolve(&self, channel: &Channel, pattern: &str, allow_failed: bool) -> anyhow::Result<Remote>;
    fn download(
        &self,
        url: &str,
        accept: &str,
        dest: &Path,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(u64, Option<u64>),
    ) -> anyhow::Result<()>;
    fn fetch_small(&self, url: &str, accept: &str) -> anyhow::Result<Vec<u8>>;
    fn icon(&self) -> anyhow::Result<Option<Icon>>;
}

pub fn forge(source: &Source, token: Option<String>) -> Box<dyn Forge> {
    match source {
        Source::Github { owner, repo } => Box::new(github::GitHub::new(owner, repo, token)),
    }
}

/// The image type of `bytes`, by its first bytes.
pub fn image_ext(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("jpg")
    } else {
        let head = String::from_utf8_lossy(&bytes[..bytes.len().min(2048)]).to_ascii_lowercase();
        head.contains("<svg").then_some("svg")
    }
}

/// The SHA-256 for `file_name` in a checksum file: `hash  name` lines
/// (sha256sum's format, `*name` too), or a lone hash.
pub fn find_checksum(text: &str, file_name: &str) -> Option<String> {
    let is_hash = |s: &str| s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit());
    let mut lone = None;
    let mut lines = 0;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        lines += 1;
        let mut parts = line.split_whitespace();
        let first = parts.next().unwrap_or("");
        let rest: Vec<&str> = parts.collect();
        if is_hash(first) {
            if rest.is_empty() {
                lone = Some(first.to_ascii_lowercase());
            } else {
                let name = rest.join(" ");
                let name = name.trim_start_matches('*');
                let base = name.rsplit('/').next().unwrap_or(name);
                if base == file_name {
                    return Some(first.to_ascii_lowercase());
                }
            }
        }
    }
    if lines == 1 {
        lone
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksums() {
        let h = "a".repeat(64);
        let g = "b".repeat(64);
        let text = format!("{h}  app.tar.gz\n{g} *other.zip\n");
        assert_eq!(find_checksum(&text, "app.tar.gz"), Some(h.clone()));
        assert_eq!(find_checksum(&text, "other.zip"), Some(g));
        assert_eq!(find_checksum(&text, "none"), None);
        assert_eq!(find_checksum(&format!("{h}\n"), "x"), Some(h.clone()));
        assert_eq!(find_checksum(&format!("{h}  dist/app.tar.gz"), "app.tar.gz"), Some(h));
    }

    #[test]
    fn image_types() {
        assert_eq!(image_ext(b"\x89PNG\r\n\x1a\nxxxx"), Some("png"));
        assert_eq!(image_ext(b"<?xml version=\"1.0\"?><svg xmlns="), Some("svg"));
        assert_eq!(image_ext(b"GIF89a"), None);
    }
}
