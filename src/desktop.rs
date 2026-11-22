//! Desktop entries: reading a field, pointing an AppImage's own entry at
//! where it was delivered, and writing one for an app that has none. Also
//! Yoink's window system, and the hidden entry that gives its window an icon on Wayland.

use crate::config::Dirs;
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
pub fn select_backend(native_wayland: bool, dirs: &Dirs) {
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
    refresh_own_entry(dirs, on_wayland && (!has_x11 || native_wayland));
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

/// The file name of Yoink's entry, in the folder set in Options.
const OWN_ENTRY_NAME: &str = "yoink.desktop";

/// An icon's file name: `stem`, a short hash of the icon's bytes, and `ext`
/// (`chippy-1a2b3c4d.svg`). Desktops (KDE especially) cache icons by path,
/// so a changed icon written under the old name keeps showing the old
/// picture; a new name for new contents sidesteps the cache.
pub fn icon_file_name(stem: &str, bytes: &[u8], ext: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{stem}-{}.{ext}", hex::encode(&Sha256::digest(bytes)[..4]))
}

/// Whether `name` is the file name of an icon Yoink wrote for itself: this
/// version's (`yoink-<hash>.svg`) or an older one's (`yoink.svg`).
fn is_own_icon_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix(APP_ID).and_then(|r| r.strip_suffix(".svg")) else {
        return false;
    };
    rest.is_empty()
        || rest
            .strip_prefix('-')
            .is_some_and(|h| h.len() == 8 && h.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn data_home() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
}

/// Where Yoink writes its entry and icon: the Desktop entries and Icons
/// folders set in Options, as for every app it manages.
fn own_paths(dirs: &Dirs) -> anyhow::Result<(PathBuf, PathBuf)> {
    Ok((
        dirs.path("desktop")?.join(OWN_ENTRY_NAME),
        dirs.path("icons")?.join(icon_file_name(APP_ID, OWN_ICON, "svg")),
    ))
}

/// What Yoink is run as: its AppImage, or else this executable.
fn own_exe() -> Option<PathBuf> {
    crate::plan::running_exe()
}

/// Every place a `yoink.desktop` may be: the folder set in Options first,
/// then where desktops look (the user's folder, then the system's).
fn entry_candidates(dirs: &Dirs) -> Vec<PathBuf> {
    let data_dirs =
        std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    let mut out: Vec<PathBuf> = Vec::new();
    let configured = own_paths(dirs).ok().map(|p| p.0);
    let searched = data_home()
        .into_iter()
        .chain(data_dirs.split(':').filter(|d| !d.is_empty()).map(PathBuf::from))
        .map(|d| d.join("applications").join(OWN_ENTRY_NAME));
    for p in configured.into_iter().chain(searched) {
        if !out.contains(&p) {
            out.push(p);
        }
    }
    out
}

fn is_own(path: &Path) -> bool {
    std::fs::read_to_string(path).is_ok_and(|t| field(&t, SELF_MARK).is_some())
}

/// Finds `yoink.desktop` (in the folder set in Options first, then where
/// desktops look) and says whose it is.
pub fn own_entry(dirs: &Dirs) -> OwnEntry {
    let Some(path) = entry_candidates(dirs).into_iter().find(|p| p.is_file()) else {
        return OwnEntry::Missing;
    };
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    if field(&text, SELF_MARK).is_none() {
        OwnEntry::System(path)
    } else if field(&text, "NoDisplay").as_deref() == Some("true") {
        OwnEntry::Hidden(path)
    } else {
        OwnEntry::Shown(path)
    }
}

/// The icon file an entry Yoink wrote for itself names, when it is one of
/// Yoink's icon files.
fn own_icon_named(text: &str) -> Option<PathBuf> {
    let icon = PathBuf::from(field(text, "Icon")?);
    let name = icon.file_name()?.to_str()?;
    (icon.is_absolute() && is_own_icon_name(name)).then_some(icon)
}

/// Deletes an entry Yoink wrote for itself and the icon it names (when
/// that is Yoink's icon file and not `keep`).
fn delete_own(entry: &Path, keep: Option<&Path>) {
    let text = std::fs::read_to_string(entry).unwrap_or_default();
    if field(&text, SELF_MARK).is_none() {
        return;
    }
    let _ = std::fs::remove_file(entry);
    if let Some(icon) = own_icon_named(&text) {
        if Some(icon.as_path()) != keep {
            let _ = std::fs::remove_file(icon);
        }
    }
}

/// Entries Yoink wrote for itself somewhere other than `keep` (an older
/// version wrote to `~/.local/share/applications`, or the folder changed).
fn delete_strays(dirs: &Dirs, keep: &Path, keep_icon: &Path) {
    for p in entry_candidates(dirs) {
        if p != keep && is_own(&p) {
            log::info!("removing Yoink's old desktop entry {}", p.display());
            delete_own(&p, Some(keep_icon));
        }
    }
}

/// Icons an entry Yoink wrote for itself left behind that `keep` replaces:
/// the one its entry names, and an older Yoink's `yoink.svg`.
pub fn stale_own_icons(dirs: &Dirs, keep: &Path) -> Vec<PathBuf> {
    let Ok((entry, _)) = own_paths(dirs) else { return Vec::new() };
    let Ok(text) = std::fs::read_to_string(&entry) else { return Vec::new() };
    if field(&text, SELF_MARK).is_none() {
        return Vec::new();
    }
    let legacy = dirs.path("icons").ok().map(|d| d.join(format!("{APP_ID}.svg")));
    let mut out: Vec<PathBuf> = Vec::new();
    for p in own_icon_named(&text).into_iter().chain(legacy) {
        if p != keep && p.is_file() && !out.contains(&p) {
            out.push(p);
        }
    }
    out
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

fn write_own_entry(dirs: &Dirs, hidden: bool) -> anyhow::Result<PathBuf> {
    use anyhow::Context;
    let exe = own_exe().context("cannot tell where Yoink is")?;
    let (entry, icon) = own_paths(dirs)?;
    let stale = stale_own_icons(dirs, &icon);
    crate::config::write_atomic(&icon, OWN_ICON)?;
    crate::config::write_atomic(&entry, own_entry_text(&exe, Some(&icon), hidden).as_bytes())?;
    for old in stale {
        let _ = std::fs::remove_file(old);
    }
    delete_strays(dirs, &entry, &icon);
    Ok(entry)
}

/// An entry Yoink wrote for itself in the applications menu (from the
/// option older versions had) and nothing asks for any more: Yoink's entry
/// now comes from its own deployment's desktop setting, like any app's.
/// Deletes it, and the icon beside it, unless `wanted` (the deployment
/// will write `yoink.desktop` over it at its next update). A hidden one,
/// written only so a Wayland compositor finds the icon, stays.
pub fn retire_menu_entry(dirs: &Dirs, wanted: bool) {
    if let OwnEntry::Shown(path) = own_entry(dirs) {
        if wanted && own_paths(dirs).is_ok_and(|(entry, _)| entry == path) {
            return;
        }
        log::info!("removing the desktop entry Yoink wrote for itself: {}", path.display());
        delete_own(&path, None);
    }
}

/// Whether `path` is an entry Yoink wrote for itself, or the icon beside
/// it: an update of Yoink's own deployment replaces them as its own.
pub fn is_own_file(dirs: &Dirs, path: &Path) -> bool {
    let Ok((entry, icon)) = own_paths(dirs) else { return false };
    if path == entry {
        is_own(path)
    } else {
        path == icon && is_own(&entry)
    }
}

/// The entry Yoink ships (and its icon), for its own deployment's desktop
/// entry: named after the window's app id, so the taskbar matches them.
pub fn own_template() -> (String, String) {
    (OWN_ENTRY_NAME.to_string(), include_str!("../packaging/yoink.desktop").to_string())
}

pub fn own_icon() -> &'static [u8] {
    OWN_ICON
}

/// At start: an entry Yoink wrote is pointed at where Yoink now runs from
/// (it moved), moved to the folders set in Options (they changed, or an
/// older Yoink wrote it to `~/.local/share`), and gets its icon back if it
/// is gone or old; and on Wayland, where the compositor shows the icon of
/// the entry matching the window's app id, a hidden one is written when
/// there is none.
fn refresh_own_entry(dirs: &Dirs, on_wayland: bool) {
    let result = match own_entry(dirs) {
        OwnEntry::Shown(path) | OwnEntry::Hidden(path) => {
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            let hidden = field(&text, "NoDisplay").as_deref() == Some("true");
            match (own_exe(), own_paths(dirs)) {
                (Some(exe), Ok((entry, icon))) => {
                    let icon_ok = std::fs::read(&icon).is_ok_and(|b| b == OWN_ICON);
                    let strays = entry_candidates(dirs).iter().any(|p| *p != entry && is_own(p));
                    if path != entry || strays || !icon_ok || text != own_entry_text(&exe, Some(&icon), hidden) {
                        write_own_entry(dirs, hidden).map(drop)
                    } else {
                        Ok(())
                    }
                }
                _ => Ok(()),
            }
        }
        OwnEntry::Missing if on_wayland => write_own_entry(dirs, true).map(drop),
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
    fn own_entry_goes_to_the_folders_set() {
        let mut dirs = Dirs::default();
        dirs.desktop = "/custom/apps".into();
        dirs.icons = "/custom/icons".into();
        let (entry, icon) = own_paths(&dirs).unwrap();
        assert_eq!(entry, Path::new("/custom/apps/yoink.desktop"));
        assert_eq!(icon, Path::new("/custom/icons").join(icon_file_name("yoink", OWN_ICON, "svg")));
        assert_eq!(entry_candidates(&dirs)[0], entry);
        dirs.desktop.clear();
        assert!(own_paths(&dirs).is_err());
    }

    #[test]
    fn icon_names_carry_a_hash() {
        let a = icon_file_name("chippy", b"<svg/>", "svg");
        assert!(a.starts_with("chippy-") && a.ends_with(".svg") && a.len() == "chippy-12345678.svg".len(), "{a}");
        assert_eq!(a, icon_file_name("chippy", b"<svg/>", "svg"));
        assert_ne!(a, icon_file_name("chippy", b"<svg />", "svg"));
        assert!(is_own_icon_name(&icon_file_name("yoink", OWN_ICON, "svg")));
        assert!(is_own_icon_name("yoink.svg"));
        assert!(!is_own_icon_name("yoink-extra.svg"));
        assert!(!is_own_icon_name("chippy.svg"));
    }

    #[test]
    fn own_icon_renders() {
        // The window icon is rendered from this SVG; one Slint cannot read
        // leaves the taskbar blank on X11.
        let img = slint::Image::load_from_svg_data(OWN_ICON).expect("icon SVG parses");
        assert_eq!((img.size().width, img.size().height), (256, 256));
    }
}
