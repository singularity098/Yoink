//! Desktop entries: reading a field, pointing an AppImage's own entry at
//! where it was delivered, and writing one for an app that has none. Also
//! Yoink's own place on the desktop: its window system and desktop entry.

use std::path::{Path, PathBuf};

/// The app id Yoink's window gives the window manager.
pub const APP_ID: &str = "yoink";

/// A key's value in the `[Desktop Entry]` group.
pub fn field(text: &str, key: &str) -> Option<String> {
    let mut in_main = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_main = line == "[Desktop Entry]";
            continue;
        }
        if !in_main {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            if k.trim() == key {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

/// Quotes a path for an `Exec` line.
fn quote(path: &Path) -> String {
    let s = path.display().to_string();
    if s.chars().any(|c| c.is_whitespace() || "\"'\\$`".contains(c)) {
        let escaped = s
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('`', "\\`")
            .replace('$', "\\$");
        format!("\"{escaped}\"")
    } else {
        s
    }
}

/// The program part of an Exec value replaced by `exe`, its arguments kept.
fn replace_program(value: &str, exe: &Path) -> String {
    let v = value.trim_start();
    let rest = if let Some(stripped) = v.strip_prefix('"') {
        // A quoted program: up to the closing quote (not escaped).
        let mut end = None;
        let mut escape = false;
        for (i, c) in stripped.char_indices() {
            if escape {
                escape = false;
            } else if c == '\\' {
                escape = true;
            } else if c == '"' {
                end = Some(i + 1);
                break;
            }
        }
        end.map(|e| &stripped[e..]).unwrap_or("")
    } else {
        v.find(char::is_whitespace).map(|i| &v[i..]).unwrap_or("")
    };
    format!("{}{}", quote(exe), rest)
}

/// An entry found in an AppImage (or package), pointed at the delivered
/// executable and icon, and marked as Yoink's.
pub fn rewrite(text: &str, exe: &Path, icon: Option<&Path>, app_id: &str) -> String {
    let mut out = String::new();
    let mut group = String::new();
    let mut marked = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            if group == "[Desktop Entry]" && !marked {
                out += &format!("X-Yoink-App={app_id}\n");
                marked = true;
            }
            group = trimmed.to_string();
            out += line;
            out.push('\n');
            continue;
        }
        let in_entry = group == "[Desktop Entry]" || group.starts_with("[Desktop Action ");
        if in_entry {
            if let Some((k, v)) = trimmed.split_once('=') {
                match k.trim() {
                    "Exec" => {
                        out += &format!("Exec={}\n", replace_program(v, exe));
                        continue;
                    }
                    "TryExec" => {
                        out += &format!("TryExec={}\n", exe.display());
                        continue;
                    }
                    "Icon" if group == "[Desktop Entry]" => {
                        if let Some(icon) = icon {
                            out += &format!("Icon={}\n", icon.display());
                            continue;
                        }
                    }
                    "X-Yoink-App" | "X-AppImage-Integrate" => continue,
                    _ => {}
                }
            }
        }
        out += line;
        out.push('\n');
    }
    if group == "[Desktop Entry]" && !marked {
        out += &format!("X-Yoink-App={app_id}\n");
    }
    out
}

/// A plain entry for an app that brings none.
pub fn generate(name: &str, comment: &str, exe: &Path, icon: Option<&Path>, app_id: &str) -> String {
    let mut s = String::from("[Desktop Entry]\nType=Application\n");
    s += &format!("Name={}\n", one_line(name));
    if !comment.trim().is_empty() {
        s += &format!("Comment={}\n", one_line(comment));
    }
    s += &format!("Exec={} %U\n", quote(exe));
    s += &format!("TryExec={}\n", exe.display());
    if let Some(icon) = icon {
        s += &format!("Icon={}\n", icon.display());
    }
    s += "Terminal=false\nCategories=Utility;\n";
    s += &format!("X-Yoink-App={app_id}\n");
    s
}

fn one_line(s: &str) -> String {
    s.replace(['\n', '\r'], " ")
}

/// Picks the winit backend: through XWayland on a Wayland desktop unless
/// native Wayland is asked for (as in Chippy), and names the window.
pub fn select_backend(native_wayland: bool) {
    let native_wayland = match std::env::var("YOINK_WAYLAND").as_deref() {
        Ok("1") | Ok("true") => true,
        Ok("0") | Ok("false") => false,
        _ => native_wayland,
    };
    let mut selector = slint::BackendSelector::new()
        .backend_name("winit".into())
        .with_winit_window_attributes_hook(window_attributes);
    let on_wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    let has_x11 = std::env::var_os("DISPLAY").is_some();
    if on_wayland && has_x11 && !native_wayland {
        use slint::winit_030::winit::platform::x11::EventLoopBuilderExtX11;
        let mut b = slint::winit_030::winit::event_loop::EventLoop::<
            slint::winit_030::SlintEvent,
        >::with_user_event();
        b.with_x11();
        selector = selector.with_winit_event_loop_builder(b);
    }
    refresh_own_entry(on_wayland && (!has_x11 || native_wayland));
    if let Err(e) = selector.select() {
        log::warn!("could not set up the window system as asked: {e}");
    }
}

fn window_attributes(
    a: slint::winit_030::winit::window::WindowAttributes,
) -> slint::winit_030::winit::window::WindowAttributes {
    use slint::winit_030::winit::platform::{wayland, x11};
    let a = wayland::WindowAttributesExtWayland::with_name(a, APP_ID, APP_ID);
    x11::WindowAttributesExtX11::with_name(a, APP_ID, "Yoink")
}

/// Where Yoink's own desktop entry stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnEntry {
    /// No `yoink.desktop` anywhere.
    Missing,
    /// One Yoink wrote, shown in the applications menu.
    Shown(PathBuf),
    /// One Yoink wrote only so the compositor finds its icon (`NoDisplay`).
    Hidden(PathBuf),
    /// One a package installed: not Yoink's to change.
    System(PathBuf),
}

/// Marks the entries Yoink writes for itself.
const SELF_MARK: &str = "X-Yoink-Self";

fn data_home() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
}

/// The entry Yoink writes, in the user's applications folder.
fn own_entry_path() -> Option<PathBuf> {
    data_home().map(|d| d.join("applications/yoink.desktop"))
}

fn own_icon_path() -> Option<PathBuf> {
    data_home().map(|d| d.join("icons/hicolor/scalable/apps/yoink.svg"))
}

/// What Yoink is run as: its AppImage, or else this executable.
fn own_exe() -> Option<PathBuf> {
    std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .or_else(|| std::env::current_exe().ok())
}

/// Finds `yoink.desktop` where desktops look (the user's folder first) and
/// says whose it is.
pub fn own_entry() -> OwnEntry {
    let data_dirs =
        std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    let found = data_home()
        .into_iter()
        .chain(data_dirs.split(':').filter(|d| !d.is_empty()).map(PathBuf::from))
        .map(|d| d.join("applications/yoink.desktop"))
        .find(|p| p.is_file());
    let Some(path) = found else { return OwnEntry::Missing };
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    if field(&text, SELF_MARK).is_none() {
        OwnEntry::System(path)
    } else if field(&text, "NoDisplay").as_deref() == Some("true") {
        OwnEntry::Hidden(path)
    } else {
        OwnEntry::Shown(path)
    }
}

/// Yoink's icon, as written next to its entry.
const OWN_ICON: &[u8] = include_bytes!("../assets/yoink-icon.svg");

/// Yoink's entry pointed at `exe` and at its icon file, hidden from the menu
/// if asked. The icon is named by its full path, not by theme name: a
/// theme lookup misses it while the user's hicolor folder has a stale icon
/// cache, which leaves the menu entry and the taskbar (which takes the icon
/// from the entry matching the window) blank.
fn own_entry_text(exe: &Path, icon: Option<&Path>, hidden: bool) -> String {
    let mut s = String::new();
    for line in include_str!("../packaging/yoink.desktop").lines() {
        if line.starts_with("Exec=") {
            s += &format!("Exec={}\nTryExec={}\n", quote(exe), exe.display());
        } else if let (true, Some(icon)) = (line.starts_with("Icon="), icon) {
            s += &format!("Icon={}\n", icon.display());
        } else {
            s += line;
            s.push('\n');
        }
    }
    if hidden {
        s += "NoDisplay=true\n";
    }
    s += &format!("{SELF_MARK}=true\n");
    s
}

fn write_own_entry(hidden: bool) -> anyhow::Result<PathBuf> {
    use anyhow::Context;
    let exe = own_exe().context("cannot tell where Yoink is")?;
    let entry = own_entry_path().context("no home folder")?;
    let icon = own_icon_path().context("no home folder")?;
    if let Some(dir) = icon.parent() {
        std::fs::create_dir_all(dir)?;
    }
    crate::config::write_atomic(&icon, OWN_ICON)?;
    if let Some(dir) = entry.parent() {
        std::fs::create_dir_all(dir)?;
    }
    crate::config::write_atomic(&entry, own_entry_text(&exe, Some(&icon), hidden).as_bytes())?;
    Ok(entry)
}

/// Puts Yoink in the applications menu (with its icon), pointed at where it
/// runs from.
pub fn install_own_entry() -> anyhow::Result<PathBuf> {
    if let OwnEntry::System(path) = own_entry() {
        anyhow::bail!("{} is already installed by a package", path.display());
    }
    write_own_entry(false)
}

/// Takes Yoink out of the applications menu: deletes the entry and icon it
/// wrote (never a package's).
pub fn remove_own_entry() -> anyhow::Result<()> {
    match own_entry() {
        OwnEntry::System(path) => {
            anyhow::bail!("{} was installed by a package, not by Yoink", path.display())
        }
        OwnEntry::Missing => Ok(()),
        OwnEntry::Shown(path) | OwnEntry::Hidden(path) => {
            std::fs::remove_file(&path)?;
            if let Some(icon) = own_icon_path() {
                let _ = std::fs::remove_file(icon);
            }
            Ok(())
        }
    }
}

/// At start: an entry Yoink wrote is pointed at where Yoink now runs from
/// (the AppImage moved) and gets its icon back if it is gone or old, and on
/// Wayland, where the compositor shows the
/// icon of the entry matching the window's app id, a hidden one is written
/// when there is none.
fn refresh_own_entry(on_wayland: bool) {
    let result = match own_entry() {
        OwnEntry::Shown(path) | OwnEntry::Hidden(path) => {
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            let hidden = field(&text, "NoDisplay").as_deref() == Some("true");
            let icon = own_icon_path();
            let icon_ok = icon
                .as_deref()
                .is_some_and(|p| std::fs::read(p).is_ok_and(|b| b == OWN_ICON));
            match own_exe() {
                Some(exe) if !icon_ok || text != own_entry_text(&exe, icon.as_deref(), hidden) => {
                    write_own_entry(hidden).map(drop)
                }
                _ => Ok(()),
            }
        }
        OwnEntry::Missing if on_wayland => write_own_entry(true).map(drop),
        _ => Ok(()),
    };
    if let Err(e) = result {
        log::warn!("could not update Yoink's desktop entry: {e:#}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENTRY: &str = "[Desktop Entry]\nName=Chippy\nExec=chippy %f\nIcon=chippy\nTryExec=chippy\nX-AppImage-Integrate=false\n\n[Desktop Action New]\nName=New\nExec=\"/old path/chippy\" --new\n";

    #[test]
    fn reads_fields() {
        assert_eq!(field(ENTRY, "Icon").as_deref(), Some("chippy"));
        assert_eq!(field(ENTRY, "Nope"), None);
    }

    #[test]
    fn rewrites_exec_and_icon() {
        let out = rewrite(
            ENTRY,
            Path::new("/home/u/Applications/Chippy x.AppImage"),
            Some(Path::new("/home/u/.local/share/icons/chippy.svg")),
            "chippy",
        );
        assert!(out.contains("Exec=\"/home/u/Applications/Chippy x.AppImage\" %f\n"), "{out}");
        assert!(out.contains("Exec=\"/home/u/Applications/Chippy x.AppImage\" --new\n"), "{out}");
        assert!(out.contains("Icon=/home/u/.local/share/icons/chippy.svg\n"));
        assert!(out.contains("X-Yoink-App=chippy\n"));
        assert!(!out.contains("X-AppImage-Integrate"));
        assert_eq!(out.matches("X-Yoink-App").count(), 1);
    }

    #[test]
    fn generates() {
        let out = generate("My Tool", "Does things", Path::new("/b/tool"), None, "x");
        assert!(out.starts_with("[Desktop Entry]\nType=Application\nName=My Tool\n"));
        assert!(out.contains("Exec=/b/tool %U\n"));
    }

    #[test]
    fn own_entry_is_marked() {
        let icon = Path::new("/h/.local/share/icons/hicolor/scalable/apps/yoink.svg");
        let shown = own_entry_text(Path::new("/a b/Yoink.AppImage"), Some(icon), false);
        assert!(shown.contains("Exec=\"/a b/Yoink.AppImage\"\n"), "{shown}");
        assert!(shown.contains("TryExec=/a b/Yoink.AppImage\n"));
        assert_eq!(field(&shown, SELF_MARK).as_deref(), Some("true"));
        assert_eq!(field(&shown, "Icon").as_deref(), Some(icon.to_str().unwrap()));
        assert_eq!(shown.matches("Icon=").count(), 1);
        assert_eq!(field(&shown, "NoDisplay"), None);
        let hidden = own_entry_text(Path::new("/y"), None, true);
        assert_eq!(field(&hidden, "Icon").as_deref(), Some("yoink"));
        assert_eq!(field(&hidden, "NoDisplay").as_deref(), Some("true"));
    }

    #[test]
    fn own_icon_renders() {
        // The window icon is rendered from this SVG; one Slint cannot read
        // leaves the taskbar blank on X11.
        let img = slint::Image::load_from_svg_data(OWN_ICON).expect("icon SVG parses");
        assert_eq!((img.size().width, img.size().height), (256, 256));
    }
}
