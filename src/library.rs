//! The apps Yoink manages (`~/.local/share/yoink/library.json`): where each
//! comes from, its deployments, and what each deployment last installed.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::config::{data_dir, write_atomic};

/// Where an app's files are published. Only GitHub for now; other forges
/// (or plain URLs) become further variants.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Source {
    Github { owner: String, repo: String },
}

impl Source {
    pub fn label(&self) -> String {
        match self {
            Source::Github { owner, repo } => format!("{owner}/{repo}"),
        }
    }

    pub fn web_url(&self) -> String {
        match self {
            Source::Github { owner, repo } => format!("https://github.com/{owner}/{repo}"),
        }
    }

    /// `owner/repo`, `github.com/owner/repo` or a GitHub URL (with or
    /// without `.git` and anything after the repository).
    pub fn parse(text: &str) -> Option<Source> {
        let mut s = text.trim().trim_end_matches('/');
        for prefix in ["https://", "http://", "git@"] {
            s = s.strip_prefix(prefix).unwrap_or(s);
        }
        for prefix in ["www.github.com/", "github.com/", "github.com:"] {
            s = s.strip_prefix(prefix).unwrap_or(s);
        }
        let mut parts = s.split('/');
        let owner = parts.next()?.trim();
        let repo = parts.next()?.trim().trim_end_matches(".git");
        let ok = |p: &str| {
            !p.is_empty()
                && p.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        };
        if !ok(owner) || !ok(repo) {
            return None;
        }
        Some(Source::Github {
            owner: owner.to_string(),
            repo: repo.to_string(),
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Channel {
    /// The Releases section: the newest release (or pre-release, if asked).
    Releases {
        #[serde(default)]
        prerelease: bool,
    },
    /// Builds from the Actions section: the newest run of `workflow` (its
    /// file name; empty for any) on `branch` (empty for the default one).
    Actions {
        #[serde(default)]
        workflow: String,
        #[serde(default)]
        branch: String,
    },
}

impl Channel {
    pub fn label(&self) -> String {
        match self {
            Channel::Releases { prerelease: false } => "Releases".into(),
            Channel::Releases { prerelease: true } => "Releases + pre-releases".into(),
            Channel::Actions { workflow, branch } => {
                let mut s = "Actions".to_string();
                if !workflow.is_empty() {
                    s += &format!(" · {workflow}");
                }
                if !branch.is_empty() {
                    s += &format!(" · {branch}");
                }
                s
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    AppImage,
    Binary,
    Plugin,
    Apk,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::AppImage, Kind::Binary, Kind::Plugin, Kind::Apk];

    pub fn label(self) -> &'static str {
        match self {
            Kind::AppImage => "AppImage",
            Kind::Binary => "Binary",
            Kind::Plugin => "Plugin",
            Kind::Apk => "APK",
        }
    }

    pub fn index(self) -> usize {
        Kind::ALL.iter().position(|k| *k == self).unwrap_or(0)
    }
}

/// Which plugin formats a plugin deployment delivers.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Formats {
    pub vst2: bool,
    pub vst3: bool,
    pub lv2: bool,
    pub clap: bool,
}

impl Default for Formats {
    fn default() -> Self {
        Formats {
            vst2: true,
            vst3: true,
            lv2: true,
            clap: true,
        }
    }
}

/// What a deployment last installed.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Installed {
    /// What identifies the build (release tag and asset, or run and
    /// artifact): a different key upstream is an update.
    pub key: String,
    /// The version as shown.
    pub version: String,
    /// Every file and folder it put in place; the next version's deployment
    /// removes those it does not put back, and removing the app removes them.
    pub files: Vec<PathBuf>,
    /// Seconds since 1970.
    pub when: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Deployment {
    pub id: String,
    pub channel: Channel,
    pub kind: Kind,
    /// The release asset or Actions artifact to download (a glob).
    pub pattern: String,
    /// The file used inside the package when it holds several candidates
    /// (the AppImage, the APK, the executable), a glob of its path.
    #[serde(default)]
    pub inner: String,
    /// The VST2 files picked inside a plugin package (globs of their paths);
    /// VST2 plugins are plain .so files, so they are picked by hand.
    #[serde(default)]
    pub vst2: Vec<String>,
    #[serde(default)]
    pub formats: Formats,
    /// The name of the executable in the bin folder (empty: its own name).
    #[serde(default)]
    pub install_name: String,
    /// Write a .desktop file (AppImages and binaries).
    #[serde(default)]
    pub desktop: bool,
    #[serde(default)]
    pub installed: Option<Installed>,
}

impl Deployment {
    pub fn new(id: String, channel: Channel, kind: Kind, pattern: String) -> Deployment {
        Deployment {
            id,
            channel,
            kind,
            pattern,
            inner: String::new(),
            vst2: Vec::new(),
            formats: Formats::default(),
            install_name: String::new(),
            desktop: matches!(kind, Kind::AppImage),
            installed: None,
        }
    }

    pub fn label(&self) -> String {
        format!("{} · {}", self.kind.label(), self.channel.label())
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum IconSource {
    #[default]
    None,
    /// The repository owner's avatar.
    Avatar,
    /// An icon file found in the repository.
    Repo,
    /// The icon inside the AppImage.
    AppImage,
    /// Picked by hand: never replaced.
    Custom,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct App {
    pub id: String,
    pub name: String,
    pub source: Source,
    /// The icon file's name in Yoink's icons folder.
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub icon_source: IconSource,
    #[serde(default)]
    pub description: String,
    pub deployments: Vec<Deployment>,
}

impl App {
    /// A file-name friendly form of the name: `My App` → `my-app`.
    pub fn slug(&self) -> String {
        slug(&self.name)
    }

    pub fn icon_path(&self) -> Option<PathBuf> {
        self.icon.as_ref().map(|f| icons_dir().join(f))
    }

    pub fn next_deployment_id(&self) -> String {
        let n = self
            .deployments
            .iter()
            .filter_map(|d| d.id.parse::<u32>().ok())
            .max()
            .unwrap_or(0);
        (n + 1).to_string()
    }
}

pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() {
        "app".into()
    } else {
        out
    }
}

pub fn icons_dir() -> PathBuf {
    data_dir().join("icons")
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Library {
    pub apps: Vec<App>,
}

impl Library {
    fn path() -> PathBuf {
        data_dir().join("library.json")
    }

    pub fn load() -> Library {
        let path = Self::path();
        match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
                // Never overwrite a library that could not be read.
                let backup = path.with_extension("json.unreadable");
                let _ = std::fs::copy(&path, &backup);
                log::error!(
                    "{}: {e}; a copy was kept as {}",
                    path.display(),
                    backup.display()
                );
                Library::default()
            }),
            Err(_) => Library::default(),
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let text = serde_json::to_string_pretty(self)?;
        write_atomic(&Self::path(), text.as_bytes())
    }

    pub fn app(&self, id: &str) -> Option<&App> {
        self.apps.iter().find(|a| a.id == id)
    }

    pub fn app_mut(&mut self, id: &str) -> Option<&mut App> {
        self.apps.iter_mut().find(|a| a.id == id)
    }

    /// A new app id from the source: `owner-repo`, numbered if taken.
    pub fn new_id(&self, source: &Source) -> String {
        let base = slug(&source.label());
        let mut id = base.clone();
        let mut n = 2;
        while self.app(&id).is_some() {
            id = format!("{base}-{n}");
            n += 1;
        }
        id
    }

    /// The app and deployment that installed `path`, if any.
    pub fn owner_of(&self, path: &std::path::Path) -> Option<(&App, &Deployment)> {
        self.apps.iter().find_map(|a| {
            a.deployments.iter().find_map(|d| {
                d.installed
                    .as_ref()
                    .filter(|i| i.files.iter().any(|f| f == path))
                    .map(|_| (a, d))
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sources() {
        let want = Some(Source::Github {
            owner: "singularity098".into(),
            repo: "Chippy".into(),
        });
        assert_eq!(Source::parse("singularity098/Chippy"), want);
        assert_eq!(Source::parse("https://github.com/singularity098/Chippy"), want);
        assert_eq!(Source::parse("github.com/singularity098/Chippy.git"), want);
        assert_eq!(
            Source::parse("https://github.com/singularity098/Chippy/releases/tag/v1"),
            want
        );
        assert_eq!(Source::parse("git@github.com:singularity098/Chippy.git"), want);
        assert_eq!(Source::parse("nope"), None);
        assert_eq!(Source::parse("a b/c"), None);
    }

    #[test]
    fn slugs() {
        assert_eq!(slug("My App"), "my-app");
        assert_eq!(slug("Surge XT (Nightly)"), "surge-xt-nightly");
        assert_eq!(slug("!!"), "app");
    }

    #[test]
    fn library_round_trip() {
        let mut d = Deployment::new(
            "1".into(),
            Channel::Actions {
                workflow: "ci.yml".into(),
                branch: String::new(),
            },
            Kind::Plugin,
            "*.zip".into(),
        );
        d.installed = Some(Installed {
            key: "k".into(),
            version: "v".into(),
            files: vec!["/a".into()],
            when: 1,
        });
        let lib = Library {
            apps: vec![App {
                id: "x".into(),
                name: "X".into(),
                source: Source::parse("a/b").unwrap(),
                icon: None,
                icon_source: IconSource::None,
                description: String::new(),
                deployments: vec![d],
            }],
        };
        let text = serde_json::to_string(&lib).unwrap();
        let back: Library = serde_json::from_str(&text).unwrap();
        assert_eq!(lib, back);
        assert!(back.owner_of(std::path::Path::new("/a")).is_some());
    }
}
