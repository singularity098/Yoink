//! Settings (`~/.config/yoink/settings.toml`) and where Yoink keeps its
//! own files. `YOINK_HOME` moves all of them below one folder (tests, or a
//! second setup).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The folders packages are deployed to. Each is a path as typed (`~`
/// allowed); empty means not set.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Dirs {
    pub appimage: String,
    pub bin: String,
    pub lib: String,
    pub share: String,
    pub package: String,
    pub vst2: String,
    pub vst3: String,
    pub lv2: String,
    pub clap: String,
    pub apk: String,
    pub desktop: String,
    pub icons: String,
}

impl Default for Dirs {
    fn default() -> Self {
        Dirs {
            appimage: "~/Applications".into(),
            bin: "~/.local/bin".into(),
            lib: "~/.local/lib".into(),
            share: "~/.local/share".into(),
            package: "~/.local/share/yoink/packages".into(),
            vst2: "~/.vst".into(),
            vst3: "~/.vst3".into(),
            lv2: "~/.lv2".into(),
            clap: "~/.clap".into(),
            apk: String::new(),
            desktop: "~/.local/share/applications".into(),
            icons: "~/.local/share/icons".into(),
        }
    }
}

/// Every folder setting: id, label, what it is for.
pub const DIR_INFO: &[(&str, &str, &str)] = &[
    ("appimage", "AppImages", "AppImages are delivered here."),
    ("bin", "Executables (bin)", "Single executables, and the bin/ of packages laid out as bin, lib, share."),
    ("lib", "Libraries (lib)", "The lib/ of packages laid out as bin, lib, share."),
    ("share", "Shared data (share)", "The share/ of packages laid out as bin, lib, share."),
    ("package", "Packages", "The archives that binaries and plugins came in are kept here."),
    ("vst3", "VST3 plugins", "Where .vst3 bundles go."),
    ("lv2", "LV2 plugins", "Where .lv2 bundles go."),
    ("clap", "CLAP plugins", "Where .clap plugins go."),
    ("vst2", "VST2 plugins", "Where the VST2 .so files you pick go."),
    ("apk", "Android APKs", "APKs are saved here, for syncing to your phone."),
    ("desktop", "Desktop entries", "Where .desktop files are written."),
    ("icons", "Icons", "Where the icons of desktop entries are written."),
];

impl Dirs {
    pub fn get(&self, id: &str) -> &str {
        match id {
            "appimage" => &self.appimage,
            "bin" => &self.bin,
            "lib" => &self.lib,
            "share" => &self.share,
            "package" => &self.package,
            "vst2" => &self.vst2,
            "vst3" => &self.vst3,
            "lv2" => &self.lv2,
            "clap" => &self.clap,
            "apk" => &self.apk,
            "desktop" => &self.desktop,
            "icons" => &self.icons,
            _ => "",
        }
    }

    pub fn set(&mut self, id: &str, value: String) {
        let slot = match id {
            "appimage" => &mut self.appimage,
            "bin" => &mut self.bin,
            "lib" => &mut self.lib,
            "share" => &mut self.share,
            "package" => &mut self.package,
            "vst2" => &mut self.vst2,
            "vst3" => &mut self.vst3,
            "lv2" => &mut self.lv2,
            "clap" => &mut self.clap,
            "apk" => &mut self.apk,
            "desktop" => &mut self.desktop,
            "icons" => &mut self.icons,
            _ => return,
        };
        *slot = value;
    }

    /// The folder for `id`, expanded, or an error naming the setting.
    pub fn path(&self, id: &str) -> anyhow::Result<PathBuf> {
        let raw = self.get(id).trim();
        if raw.is_empty() {
            let label = DIR_INFO
                .iter()
                .find(|d| d.0 == id)
                .map(|d| d.1)
                .unwrap_or(id);
            anyhow::bail!("the {label} folder is not set (Options)");
        }
        Ok(expand(raw))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub dirs: Dirs,
    pub theme: String,
    pub check_on_startup: bool,
    /// Use an Actions build from a run that failed (other jobs failing
    /// does not always mean the file wanted is broken).
    pub allow_failed_runs: bool,
    /// A panel opens when the mouse rests on the window's edge.
    pub hover_opens: bool,
    pub native_wayland: bool,
    /// Where Yoink's own updates come from: "releases" or "actions".
    pub self_channel: String,
    pub width: f32,
    pub height: f32,
    pub maximized: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            dirs: Dirs::default(),
            theme: "Iris".into(),
            check_on_startup: true,
            allow_failed_runs: false,
            hover_opens: false,
            native_wayland: false,
            self_channel: "releases".into(),
            width: 1100.0,
            height: 760.0,
            maximized: false,
        }
    }
}

impl Settings {
    pub fn load() -> Settings {
        let path = config_dir().join("settings.toml");
        match std::fs::read_to_string(&path) {
            Ok(s) => toml::from_str(&s).unwrap_or_else(|e| {
                log::warn!("{}: {e}; using the defaults", path.display());
                Settings::default()
            }),
            Err(_) => Settings::default(),
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let text = toml::to_string_pretty(self)?;
        write_atomic(&config_dir().join("settings.toml"), text.as_bytes())
    }
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

fn yoink_home() -> Option<PathBuf> {
    std::env::var_os("YOINK_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

fn xdg(var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(fallback))
}

pub fn config_dir() -> PathBuf {
    yoink_home()
        .map(|h| h.join("config"))
        .unwrap_or_else(|| xdg("XDG_CONFIG_HOME", ".config").join("yoink"))
}

pub fn data_dir() -> PathBuf {
    yoink_home()
        .map(|h| h.join("data"))
        .unwrap_or_else(|| xdg("XDG_DATA_HOME", ".local/share").join("yoink"))
}

pub fn cache_dir() -> PathBuf {
    yoink_home()
        .map(|h| h.join("cache"))
        .unwrap_or_else(|| xdg("XDG_CACHE_HOME", ".cache").join("yoink"))
}

/// `~` and `~/…` become the home folder; `$HOME` too.
pub fn expand(path: &str) -> PathBuf {
    let path = path.trim();
    if path == "~" {
        return home();
    }
    if let Some(rest) = path.strip_prefix("~/") {
        return home().join(rest);
    }
    if let Some(rest) = path.strip_prefix("$HOME/") {
        return home().join(rest);
    }
    PathBuf::from(path)
}

/// The path with the home folder shown as `~`.
pub fn tilde(path: &Path) -> String {
    let h = home();
    match path.strip_prefix(&h) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) if h != Path::new("/") => format!("~/{}", rest.display()),
        _ => path.display().to_string(),
    }
}

/// Writes through a temporary file beside `path`, so a crash never leaves
/// half a file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("yoink-tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_home() {
        let h = home();
        assert_eq!(expand("~"), h);
        assert_eq!(expand("~/x/y"), h.join("x/y"));
        assert_eq!(expand("/opt/x"), PathBuf::from("/opt/x"));
        assert_eq!(tilde(&h.join("a")), "~/a");
    }

    #[test]
    fn settings_round_trip() {
        let s = Settings::default();
        let t = toml::to_string_pretty(&s).unwrap();
        let back: Settings = toml::from_str(&t).unwrap();
        assert_eq!(s, back);
        // Missing fields take their defaults.
        let partial: Settings = toml::from_str("theme = \"Ember\"\n[dirs]\nbin = \"/b\"\n").unwrap();
        assert_eq!(partial.theme, "Ember");
        assert_eq!(partial.dirs.bin, "/b");
        assert_eq!(partial.dirs.lib, Dirs::default().lib);
    }

    #[test]
    fn unset_dir_is_an_error() {
        let d = Dirs::default();
        let e = d.path("apk").unwrap_err().to_string();
        assert!(e.contains("Android APKs"), "{e}");
    }
}
