//! HTTPS: one agent for everything (rustls with the system's certificates,
//! the proxy from the environment), JSON requests, and downloads that
//! report progress and follow redirects by hand, so a token sent to GitHub
//! never travels on to the storage host a download redirects to.

use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{anyhow, bail, Context};

pub const USER_AGENT: &str = concat!("yoink/", env!("CARGO_PKG_VERSION"));

fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::AgentBuilder::new()
            .user_agent(USER_AGENT)
            .try_proxy_from_env(true)
            .timeout_connect(Duration::from_secs(20))
            .timeout_read(Duration::from_secs(60))
            .redirects(0)
            .build()
    })
}

/// A request's credentials: sent only to these hosts.
#[derive(Clone, Default)]
pub struct Auth {
    pub token: Option<String>,
    pub hosts: &'static [&'static str],
}

impl Auth {
    fn applies(&self, url: &str) -> Option<&str> {
        let host = url
            .split("://")
            .nth(1)?
            .split(['/', ':', '?'])
            .next()?
            .to_ascii_lowercase();
        let token = self.token.as_deref()?;
        self.hosts.iter().any(|h| *h == host).then_some(token)
    }
}

/// A GET that follows up to 8 redirects, sending the token only where it
/// belongs. Returns the final response (status < 400) or an error saying
/// what the server answered.
pub fn get(url: &str, accept: &str, auth: &Auth) -> anyhow::Result<ureq::Response> {
    let mut url = url.to_string();
    for _ in 0..8 {
        let mut req = agent()
            .get(&url)
            .set("Accept", accept)
            .set("X-GitHub-Api-Version", "2022-11-28");
        if let Some(token) = auth.applies(&url) {
            req = req.set("Authorization", &format!("Bearer {token}"));
        }
        match req.call() {
            Ok(resp) if (300..400).contains(&resp.status()) => {
                let next = resp
                    .header("Location")
                    .ok_or_else(|| anyhow!("{url}: redirect without a location"))?;
                url = if next.starts_with("http") {
                    next.to_string()
                } else {
                    // A path on the same host.
                    let base: String = url.splitn(4, '/').take(3).collect::<Vec<_>>().join("/");
                    format!("{base}{next}")
                };
            }
            Ok(resp) => return Ok(resp),
            Err(ureq::Error::Status(code, resp)) => return Err(status_error(code, resp)),
            Err(e) => return Err(anyhow!("could not reach {}: {e}", host_of(&url))),
        }
    }
    bail!("{url}: too many redirects")
}

fn host_of(url: &str) -> &str {
    url.split("://")
        .nth(1)
        .and_then(|r| r.split('/').next())
        .unwrap_or(url)
}

/// An HTTP error worth reading: GitHub's message and, for its rate limit,
/// when it resets.
#[derive(Debug)]
pub struct HttpError {
    pub status: u16,
    pub message: String,
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for HttpError {}

fn status_error(code: u16, resp: ureq::Response) -> anyhow::Error {
    let remaining = resp.header("x-ratelimit-remaining").map(str::to_string);
    let reset = resp
        .header("x-ratelimit-reset")
        .and_then(|r| r.parse::<u64>().ok());
    let url = resp.get_url().to_string();
    let body = resp.into_string().unwrap_or_default();
    let api_message = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| v.get("message").and_then(|m| m.as_str()).map(str::to_string));
    let message = if (code == 403 || code == 429) && remaining.as_deref() == Some("0") {
        let wait = reset
            .map(|r| {
                let now = crate::now();
                format!(" (resets in {} min)", r.saturating_sub(now).div_ceil(60))
            })
            .unwrap_or_default();
        format!("GitHub's rate limit was reached{wait}; a token in Options raises it")
    } else {
        match code {
            401 => "GitHub did not accept the token (401): check it in Options".to_string(),
            404 => format!(
                "not found (404): {}",
                api_message
                    .clone()
                    .unwrap_or_else(|| "a private repository needs a token".into())
            ),
            _ => format!(
                "{} answered {code}{}",
                host_of(&url),
                api_message.map(|m| format!(": {m}")).unwrap_or_default()
            ),
        }
    };
    anyhow::Error::new(HttpError {
        status: code,
        message,
    })
}

pub fn get_json<T: serde::de::DeserializeOwned>(url: &str, auth: &Auth) -> anyhow::Result<T> {
    let resp = get(url, "application/vnd.github+json", auth)?;
    let text = resp
        .into_string()
        .with_context(|| format!("reading {url}"))?;
    serde_json::from_str(&text).with_context(|| format!("unexpected answer from {url}"))
}

pub fn get_bytes(url: &str, accept: &str, auth: &Auth, limit: u64) -> anyhow::Result<Vec<u8>> {
    let resp = get(url, accept, auth)?;
    let mut out = Vec::new();
    resp.into_reader().take(limit).read_to_end(&mut out)?;
    Ok(out)
}

/// Downloads to `dest` (through `dest.part`), calling `progress(done,
/// total)` as it goes. Setting `cancel` stops it.
pub fn download(
    url: &str,
    accept: &str,
    auth: &Auth,
    dest: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64, Option<u64>),
) -> anyhow::Result<()> {
    let resp = get(url, accept, auth)?;
    let total = resp
        .header("Content-Length")
        .and_then(|l| l.parse::<u64>().ok());
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let part = dest.with_extension("part");
    let mut out = std::fs::File::create(&part)
        .with_context(|| format!("creating {}", part.display()))?;
    let mut reader = resp.into_reader();
    let mut buf = vec![0u8; 256 * 1024];
    let mut done = 0u64;
    progress(0, total);
    loop {
        if cancel.load(Ordering::Relaxed) {
            drop(out);
            let _ = std::fs::remove_file(&part);
            bail!("cancelled");
        }
        let n = reader.read(&mut buf).context("downloading")?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])?;
        done += n as u64;
        progress(done, total);
    }
    out.sync_all()?;
    drop(out);
    if let Some(t) = total {
        if t != done {
            let _ = std::fs::remove_file(&part);
            bail!("the download was cut short ({done} of {t} bytes)");
        }
    }
    std::fs::rename(&part, dest)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_only_for_its_hosts() {
        let auth = Auth {
            token: Some("t".into()),
            hosts: &["api.github.com"],
        };
        assert_eq!(auth.applies("https://api.github.com/repos/a/b"), Some("t"));
        assert_eq!(auth.applies("https://API.github.com:443/x"), Some("t"));
        assert_eq!(auth.applies("https://objects.githubusercontent.com/x"), None);
        assert_eq!(auth.applies("https://api.github.com.evil.com/x"), None);
    }
}
