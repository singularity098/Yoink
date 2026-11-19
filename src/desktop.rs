//! Desktop entries: reading a field, pointing an AppImage's own entry at
//! where it was delivered, and writing one for an app that has none. Also
//! Yoink's own place on the desktop: its window system and desktop entry.

use std::path::Path;

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
    } else if on_wayland {
        install_own_entry();
    }
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

/// On Wayland the compositor shows the icon of the desktop entry matching
/// the window's app id: without an installed `yoink.desktop` (an AppImage,
/// `cargo run`), one is written.
fn install_own_entry() {
    use std::path::PathBuf;
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")));
    let Some(data_home) = data_home else { return };
    let data_dirs =
        std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    let installed = std::iter::once(data_home.clone())
        .chain(data_dirs.split(':').filter(|d| !d.is_empty()).map(PathBuf::from))
        .any(|d| d.join("applications/yoink.desktop").is_file());
    if installed {
        return;
    }
    let exe = std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .or_else(|| std::env::current_exe().ok());
    let Some(exe) = exe else { return };
    let icon_dir = data_home.join("icons/hicolor/scalable/apps");
    let apps = data_home.join("applications");
    let entry = include_str!("../packaging/yoink.desktop")
        .replace("Exec=yoink", &format!("Exec={}", quote(&exe)));
    let result = (|| -> std::io::Result<()> {
        std::fs::create_dir_all(&icon_dir)?;
        std::fs::write(icon_dir.join("yoink.svg"), include_bytes!("../assets/yoink-icon.svg"))?;
        std::fs::create_dir_all(&apps)?;
        std::fs::write(apps.join("yoink.desktop"), entry)
    })();
    if let Err(e) = result {
        log::warn!("could not install a desktop entry: {e}");
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
}
