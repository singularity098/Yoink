//! GitHub: releases, Actions runs and their artifacts, and icons from the
//! repository.

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;

use anyhow::{bail, Context};
use serde::Deserialize;

use super::{image_ext, Candidate, ChecksumSource, Forge, Icon, Offer, Remote, RepoInfo};
use crate::http::{self, Auth};
use crate::library::{Channel, IconSource};
use crate::pattern::glob_match;

const API: &str = "https://api.github.com";
/// Hosts the token may be sent to.
const HOSTS: &[&str] = &["api.github.com"];
/// How far back to look for a release or run with the file wanted.
const LOOK_BACK: usize = 15;

pub struct GitHub {
    owner: String,
    repo: String,
    auth: Auth,
    repo_json: Mutex<Option<RepoJson>>,
}

#[derive(Deserialize, Clone)]
struct RepoJson {
    name: String,
    #[serde(default)]
    description: Option<String>,
    default_branch: String,
    owner: OwnerJson,
}

#[derive(Deserialize, Clone)]
struct OwnerJson {
    avatar_url: String,
}

#[derive(Deserialize)]
struct ReleaseJson {
    tag_name: String,
    #[serde(default)]
    name: Option<String>,
    draft: bool,
    prerelease: bool,
    #[serde(default)]
    published_at: Option<String>,
    html_url: String,
    assets: Vec<AssetJson>,
}

#[derive(Deserialize)]
struct AssetJson {
    id: u64,
    name: String,
    size: u64,
    url: String,
    browser_download_url: String,
    #[serde(default)]
    updated_at: String,
}

#[derive(Deserialize)]
struct WorkflowsJson {
    workflows: Vec<WorkflowJson>,
}

#[derive(Deserialize)]
struct WorkflowJson {
    path: String,
    state: String,
}

#[derive(Deserialize)]
struct RunsJson {
    workflow_runs: Vec<RunJson>,
}

#[derive(Deserialize)]
struct RunJson {
    id: u64,
    run_number: u64,
    head_sha: String,
    status: String,
    #[serde(default)]
    conclusion: Option<String>,
    #[serde(default)]
    name: Option<String>,
    html_url: String,
    created_at: String,
}

#[derive(Deserialize)]
struct ArtifactsJson {
    artifacts: Vec<ArtifactJson>,
}

#[derive(Deserialize)]
struct ArtifactJson {
    id: u64,
    name: String,
    size_in_bytes: u64,
    expired: bool,
    archive_download_url: String,
}

#[derive(Deserialize)]
struct TreeJson {
    tree: Vec<TreeEntry>,
}

#[derive(Deserialize)]
struct TreeEntry {
    path: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    size: Option<u64>,
}

impl GitHub {
    pub fn new(owner: &str, repo: &str, token: Option<String>) -> GitHub {
        GitHub {
            owner: owner.to_string(),
            repo: repo.to_string(),
            auth: Auth {
                token,
                hosts: HOSTS,
            },
            repo_json: Mutex::new(None),
        }
    }

    fn api(&self, path: &str) -> String {
        format!("{API}/repos/{}/{}{path}", self.owner, self.repo)
    }

    fn repo_json(&self) -> anyhow::Result<RepoJson> {
        if let Some(r) = self.repo_json.lock().unwrap().clone() {
            return Ok(r);
        }
        let r: RepoJson = http::get_json(&self.api(""), &self.auth)
            .with_context(|| format!("{}/{}", self.owner, self.repo))?;
        *self.repo_json.lock().unwrap() = Some(r.clone());
        Ok(r)
    }

    fn releases(&self) -> anyhow::Result<Vec<ReleaseJson>> {
        http::get_json(&self.api(&format!("/releases?per_page={LOOK_BACK}")), &self.auth)
    }

    fn runs(&self, workflow: &str, branch: &str) -> anyhow::Result<Vec<RunJson>> {
        let branch = if branch.is_empty() {
            self.repo_json()?.default_branch
        } else {
            branch.to_string()
        };
        let base = if workflow.is_empty() {
            self.api("/actions/runs")
        } else {
            self.api(&format!("/actions/workflows/{}/runs", enc(workflow)))
        };
        let url = format!("{base}?branch={}&per_page={LOOK_BACK}", enc(&branch));
        Ok(http::get_json::<RunsJson>(&url, &self.auth)?.workflow_runs)
    }

    fn artifacts(&self, run: u64) -> anyhow::Result<Vec<ArtifactJson>> {
        let url = self.api(&format!("/actions/runs/{run}/artifacts?per_page=100"));
        Ok(http::get_json::<ArtifactsJson>(&url, &self.auth)?.artifacts)
    }

    fn release_wanted(r: &ReleaseJson, prerelease: bool) -> bool {
        !r.draft && (prerelease || !r.prerelease)
    }

    fn checksum_for(&self, r: &ReleaseJson, asset: &AssetJson) -> Option<ChecksumSource> {
        let lower = asset.name.to_ascii_lowercase();
        let own = [format!("{lower}.sha256"), format!("{lower}.sha256sum")];
        let shared = |n: &str| {
            n.contains("sha256") && (n.contains("sum") || n.ends_with(".txt"))
                || n == "checksums.txt"
                || n.ends_with("checksums.txt")
        };
        let found = r
            .assets
            .iter()
            .find(|a| own.contains(&a.name.to_ascii_lowercase()))
            .or_else(|| {
                r.assets
                    .iter()
                    .find(|a| a.id != asset.id && shared(&a.name.to_ascii_lowercase()))
            })?;
        Some(self.asset_source(found))
    }

    /// How to download an asset: through the API (works for private
    /// repositories) with a token, the public link without.
    fn asset_source(&self, a: &AssetJson) -> ChecksumSource {
        if self.auth.token.is_some() {
            ChecksumSource {
                url: a.url.clone(),
                accept: "application/octet-stream".into(),
            }
        } else {
            ChecksumSource {
                url: a.browser_download_url.clone(),
                accept: "application/octet-stream".into(),
            }
        }
    }

    fn resolve_release(&self, prerelease: bool, pattern: &str) -> anyhow::Result<Remote> {
        let releases = self.releases()?;
        let wanted: Vec<&ReleaseJson> = releases
            .iter()
            .filter(|r| Self::release_wanted(r, prerelease))
            .collect();
        if wanted.is_empty() {
            bail!(
                "no {}releases published",
                if prerelease { "" } else { "stable " }
            );
        }
        for (i, r) in wanted.iter().enumerate() {
            let Some(asset) = r.assets.iter().find(|a| glob_match(pattern, &a.name)) else {
                continue;
            };
            let src = self.asset_source(asset);
            let note = if i > 0 {
                format!(
                    "the newest release ({}) has no file matching {pattern}",
                    wanted[0].tag_name
                )
            } else {
                String::new()
            };
            let _ = &r.name;
            return Ok(Remote {
                key: format!("release:{}:{}:{}", r.tag_name, asset.id, asset.updated_at),
                version: r.tag_name.clone(),
                file_name: asset.name.clone(),
                size: asset.size,
                url: src.url,
                accept: src.accept,
                needs_token: false,
                checksum: self.checksum_for(r, asset),
                note,
                web: r.html_url.clone(),
                published: r.published_at.clone().unwrap_or_default(),
            });
        }
        bail!(
            "no file matching {pattern} in the last {} releases",
            wanted.len()
        )
    }

    fn resolve_actions(
        &self,
        workflow: &str,
        branch: &str,
        pattern: &str,
        allow_failed: bool,
    ) -> anyhow::Result<Remote> {
        let runs = self.runs(workflow, branch)?;
        if runs.is_empty() {
            bail!("no Actions runs found{}", where_(workflow, branch));
        }
        let mut in_progress: Option<&RunJson> = None;
        let mut skipped_failed: Option<&RunJson> = None;
        for run in &runs {
            let running = run.status != "completed";
            let conclusion = run.conclusion.as_deref().unwrap_or("");
            // A cancelled run (one job stuck or unable to start, say) still
            // offers the artifacts its finished jobs uploaded: an artifact
            // is only uploaded once its job's build and tests are through.
            let cancelled = conclusion == "cancelled";
            if !running
                && conclusion != "success"
                && !cancelled
                && !(allow_failed && conclusion == "failure")
            {
                if conclusion == "failure" && skipped_failed.is_none() {
                    skipped_failed = Some(run);
                }
                continue;
            }
            let artifacts = self.artifacts(run.id)?;
            let Some(a) = artifacts
                .iter()
                .find(|a| !a.expired && glob_match(pattern, &a.name))
            else {
                // Still building (this file not uploaded yet): the one
                // before it is what there is for now.
                if running && in_progress.is_none() {
                    in_progress = Some(run);
                }
                continue;
            };
            let short = &run.head_sha[..run.head_sha.len().min(7)];
            let mut notes = Vec::new();
            if let Some(p) = in_progress {
                notes.push(format!("a newer build (#{}) is in progress", p.run_number));
            }
            if running {
                notes.push(format!(
                    "from run #{}, still in progress (this file is finished)",
                    run.run_number
                ));
            }
            if conclusion == "failure" {
                notes.push(format!("run #{} failed", run.run_number));
            }
            if cancelled {
                notes.push(format!(
                    "run #{} was cancelled (this file is finished)",
                    run.run_number
                ));
            }
            let _ = &run.name;
            return Ok(Remote {
                key: format!("actions:{}:{}:{}", run.id, a.id, run.head_sha),
                version: format!("#{} ({short})", run.run_number),
                file_name: format!("{}.zip", a.name),
                size: a.size_in_bytes,
                url: a.archive_download_url.clone(),
                accept: "application/vnd.github+json".into(),
                needs_token: true,
                checksum: None,
                note: notes.join("; "),
                web: run.html_url.clone(),
                published: run.created_at.clone(),
            });
        }
        if let Some(p) = in_progress {
            bail!(
                "run #{} is in progress and {pattern} is not uploaded yet; no earlier build has it",
                p.run_number
            );
        }
        if let Some(f) = skipped_failed {
            bail!(
                "no successful run has {pattern} (run #{} failed; Options can allow failed runs)",
                f.run_number
            );
        }
        bail!(
            "no artifact matching {pattern} in the last {} runs{} (artifacts expire after 90 days)",
            runs.len(),
            where_(workflow, branch)
        )
    }

    fn workflows(&self) -> anyhow::Result<Vec<String>> {
        let w: WorkflowsJson = http::get_json(&self.api("/actions/workflows?per_page=100"), &self.auth)?;
        Ok(w.workflows
            .into_iter()
            .filter(|w| w.state == "active")
            .filter_map(|w| w.path.rsplit('/').next().map(str::to_string))
            .filter(|p| p.ends_with(".yml") || p.ends_with(".yaml"))
            .collect())
    }

    fn repo_icon(&self, branch: &str) -> anyhow::Result<Option<Icon>> {
        let url = self.api(&format!("/git/trees/{}?recursive=1", enc(branch)));
        let tree: TreeJson = http::get_json(&url, &self.auth)?;
        let repo = self.repo.to_ascii_lowercase();
        let mut cands: Vec<(i32, &TreeEntry)> = tree
            .tree
            .iter()
            .filter(|e| e.kind == "blob" && e.size.unwrap_or(0) <= 600_000)
            .filter_map(|e| icon_score(&e.path, &repo).map(|s| (s, e)))
            .collect();
        // Best first; the shorter path between equals (the plainer name).
        cands.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.path.len().cmp(&b.1.path.len())));
        // A logo with the name written beside it (a wide banner) makes a poor
        // icon: of the likeliest few, the first that is about square wins,
        // else the likeliest of them.
        let mut fallback: Option<Icon> = None;
        for (_, e) in cands.iter().take(5) {
            let url = self.api(&format!("/contents/{}?ref={}", enc_path(&e.path), enc(branch)));
            let bytes = match http::get_bytes(&url, "application/vnd.github.raw", &self.auth, 1_000_000) {
                Ok(b) => b,
                Err(err) => {
                    log::info!("icon candidate {} not fetched: {err:#}", e.path);
                    continue;
                }
            };
            let Some(ext) = image_ext(&bytes) else { continue };
            let squarish = image_aspect(&bytes).is_none_or(|r| r <= 1.3);
            let icon = Icon {
                bytes,
                ext,
                source: IconSource::Repo,
            };
            if squarish {
                return Ok(Some(icon));
            }
            fallback.get_or_insert(icon);
        }
        Ok(fallback)
    }
}

/// How far an image is from square: its longer side over its shorter
/// (None: not known). PNGs by their header, SVGs by their `viewBox` (or
/// `width` and `height`).
fn image_aspect(bytes: &[u8]) -> Option<f64> {
    let (w, h) = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") && bytes.len() >= 24 {
        let n = |i: usize| u32::from_be_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]) as f64;
        (n(16), n(20))
    } else {
        let text = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]);
        let start = text.find("<svg")?;
        let tag = &text[start..start + text[start..].find('>')?];
        let attr = |name: &str| -> Option<&str> {
            let at = tag.find(&format!(" {name}="))? + name.len() + 2;
            let quote = tag[at..].chars().next()?;
            let rest = &tag[at + 1..];
            Some(&rest[..rest.find(quote)?])
        };
        let num = |v: &str| v.trim().trim_end_matches("px").parse::<f64>().ok();
        match attr("viewBox") {
            Some(vb) => {
                let v: Vec<f64> = vb
                    .split(|c: char| c == ',' || c.is_whitespace())
                    .filter(|p| !p.is_empty())
                    .filter_map(|p| p.parse().ok())
                    .collect();
                (*v.get(2)?, *v.get(3)?)
            }
            None => (num(attr("width")?)?, num(attr("height")?)?),
        }
    };
    (w > 0.0 && h > 0.0).then(|| w.max(h) / w.min(h))
}

/// How likely a repository file is to be the app's icon (None: not one).
fn icon_score(path: &str, repo: &str) -> Option<i32> {
    let lower = path.to_ascii_lowercase();
    let (stem, ext) = lower.rsplit('/').next()?.rsplit_once('.')?;
    let ext_score = match ext {
        "svg" => 30,
        "png" => 20,
        _ => return None,
    };
    let bad = [
        "test", "node_modules", "screenshot", "example", "vendor", "third_party", "thirdparty",
        "external", "deps/", "submodule", "badge", "banner", "button", "/fonts/", "emoji",
    ];
    if bad.iter().any(|b| lower.contains(b)) {
        return None;
    }
    let mut score = ext_score;
    let named = if stem == format!("{repo}-icon") || stem == format!("{repo}_icon") {
        65
    } else if stem == repo {
        60
    } else if ["icon", "logo", "mark", "symbol"].iter().any(|w| stem.contains(w)) {
        40
    } else if stem.contains(repo) && repo.len() >= 3 {
        25
    } else {
        return None;
    };
    score += named;
    if lower.contains("icon") && lower.contains("hicolor") {
        score += 5;
    }
    for big in ["512", "256", "scalable"] {
        if lower.contains(big) {
            score += 8;
            break;
        }
    }
    if lower.contains("16x16") || lower.contains("32x32") || lower.contains("small") {
        score -= 20;
    }
    if lower.contains("assets/") || lower.contains("resources/") || lower.contains("icons/") {
        score += 5;
    }
    // Shallower paths first.
    score -= 3 * lower.matches('/').count() as i32;
    Some(score)
}

fn where_(workflow: &str, branch: &str) -> String {
    match (workflow.is_empty(), branch.is_empty()) {
        (true, true) => " on the default branch".into(),
        (false, true) => format!(" of {workflow} on the default branch"),
        (true, false) => format!(" on {branch}"),
        (false, false) => format!(" of {workflow} on {branch}"),
    }
}

/// Percent-encodes a query or path segment.
fn enc(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out += &format!("%{b:02X}");
        }
    }
    out
}

fn enc_path(s: &str) -> String {
    s.split('/').map(enc).collect::<Vec<_>>().join("/")
}

impl Forge for GitHub {
    fn info(&self) -> anyhow::Result<RepoInfo> {
        let r = self.repo_json()?;
        // Listing workflows needs nothing special, but a repository without
        // Actions answers 404: that is just none.
        let workflows = self.workflows().unwrap_or_default();
        Ok(RepoInfo {
            name: r.name,
            description: r.description.unwrap_or_default(),
            default_branch: r.default_branch,
            workflows,
        })
    }

    fn offer(&self, channel: &Channel) -> anyhow::Result<Offer> {
        match channel {
            Channel::Releases { prerelease } => {
                let releases = self.releases()?;
                let Some(r) = releases
                    .iter()
                    .find(|r| Self::release_wanted(r, *prerelease) && !r.assets.is_empty())
                else {
                    bail!("no releases with files");
                };
                Ok(Offer {
                    files: r
                        .assets
                        .iter()
                        .map(|a| Candidate {
                            name: a.name.clone(),
                            size: a.size,
                        })
                        .collect(),
                    version: r.tag_name.clone(),
                    note: String::new(),
                })
            }
            Channel::Actions { workflow, branch } => {
                let runs = self.runs(workflow, branch)?;
                for run in runs.iter().take(8) {
                    let artifacts: Vec<_> = self
                        .artifacts(run.id)?
                        .into_iter()
                        .filter(|a| !a.expired)
                        .collect();
                    if artifacts.is_empty() {
                        continue;
                    }
                    let short = &run.head_sha[..run.head_sha.len().min(7)];
                    return Ok(Offer {
                        files: artifacts
                            .into_iter()
                            .map(|a| Candidate {
                                name: a.name,
                                size: a.size_in_bytes,
                            })
                            .collect(),
                        version: format!("run #{} ({short})", run.run_number),
                        note: if run.status != "completed" {
                            "this run is still in progress: more files may come".into()
                        } else {
                            String::new()
                        },
                    });
                }
                bail!("no artifacts in the recent runs{}", where_(workflow, branch))
            }
        }
    }

    fn resolve(&self, channel: &Channel, pattern: &str, allow_failed: bool) -> anyhow::Result<Remote> {
        match channel {
            Channel::Releases { prerelease } => self.resolve_release(*prerelease, pattern),
            Channel::Actions { workflow, branch } => {
                self.resolve_actions(workflow, branch, pattern, allow_failed)
            }
        }
    }

    fn download(
        &self,
        url: &str,
        accept: &str,
        dest: &Path,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(u64, Option<u64>),
    ) -> anyhow::Result<()> {
        if url.contains("/actions/artifacts/") && self.auth.token.is_none() {
            bail!("GitHub only lets Actions builds be downloaded with a token: add one in Options");
        }
        http::download(url, accept, &self.auth, dest, cancel, progress)
    }

    fn fetch_small(&self, url: &str, accept: &str) -> anyhow::Result<Vec<u8>> {
        http::get_bytes(url, accept, &self.auth, 4_000_000)
    }

    fn icon(&self) -> anyhow::Result<Option<Icon>> {
        let r = self.repo_json()?;
        match self.repo_icon(&r.default_branch) {
            Ok(Some(icon)) => return Ok(Some(icon)),
            Ok(None) => {}
            Err(e) => log::info!("no icon from the repository: {e:#}"),
        }
        let sep = if r.owner.avatar_url.contains('?') { '&' } else { '?' };
        let url = format!("{}{sep}s=256", r.owner.avatar_url);
        let bytes = http::get_bytes(&url, "image/*", &self.auth, 2_000_000)?;
        Ok(image_ext(&bytes).map(|ext| Icon {
            bytes,
            ext,
            source: IconSource::Avatar,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aspects() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="-10 -10 579 152">"#;
        assert!((image_aspect(svg).unwrap() - 579.0 / 152.0).abs() < 1e-9);
        let sq = br#"<?xml version="1.0"?><svg width="64px" height='64'>"#;
        assert_eq!(image_aspect(sq), Some(1.0));
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        png.extend(1219u32.to_be_bytes());
        png.extend(320u32.to_be_bytes());
        assert!(image_aspect(&png).unwrap() > 3.0);
        assert_eq!(image_aspect(b"<svg>"), None);
    }

    #[test]
    fn icon_scores() {
        assert!(icon_score("assets/chippy-icon.svg", "chippy").is_some());
        assert!(icon_score("README.md", "chippy").is_none());
        assert!(icon_score("tests/icon.png", "chippy").is_none());
        let a = icon_score("assets/chippy-icon.svg", "chippy").unwrap();
        let b = icon_score("docs/img/random-logo.png", "chippy").unwrap();
        assert!(a > b);
        let big = icon_score("icons/256x256/logo.png", "x").unwrap();
        let small = icon_score("icons/16x16/logo.png", "x").unwrap();
        assert!(big > small);
    }

    #[test]
    fn encodes() {
        assert_eq!(enc("release.yml"), "release.yml");
        assert_eq!(enc("feature/x y"), "feature%2Fx%20y");
        assert_eq!(enc_path("a b/c.png"), "a%20b/c.png");
    }
}
