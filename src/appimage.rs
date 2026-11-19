//! Reading inside an AppImage (its squashfs image) without running it: the
//! desktop entry at its root and its icon.

use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use backhand::{FilesystemReader, InnerNode};

use crate::archive::appimage_offset;
use crate::forge::image_ext;

#[derive(Default)]
pub struct AppImageInfo {
    /// The desktop entry's file name and text.
    pub desktop: Option<(String, String)>,
    /// The icon's bytes and type (png or svg).
    pub icon: Option<(Vec<u8>, &'static str)>,
}

pub fn read(path: &Path) -> anyhow::Result<AppImageInfo> {
    let offset = appimage_offset(path)?;
    let file = BufReader::new(std::fs::File::open(path)?);
    let fs = FilesystemReader::from_reader_with_offset(file, offset)
        .map_err(|e| anyhow::anyhow!("reading the AppImage's image: {e}"))?;

    let norm = |p: &Path| -> PathBuf {
        PathBuf::from(p.to_string_lossy().trim_start_matches('/'))
    };
    let find = |want: &Path| fs.files().find(|n| norm(&n.fullpath) == want);
    let read_file = |want: &Path, limit: u64| -> Option<Vec<u8>> {
        // Follow links (.DirIcon usually is one) a few times.
        let mut want = want.to_path_buf();
        for _ in 0..6 {
            let node = find(&want)?;
            match &node.inner {
                InnerNode::File(f) => {
                    let mut out = Vec::new();
                    fs.file(f).reader().take(limit).read_to_end(&mut out).ok()?;
                    return Some(out);
                }
                InnerNode::Symlink(l) => {
                    let target = &l.link;
                    want = if target.is_absolute() {
                        norm(target)
                    } else {
                        let base = want.parent().unwrap_or(Path::new("")).join(target);
                        normalize(&base)?
                    };
                }
                _ => return None,
            }
        }
        None
    };

    let mut info = AppImageInfo::default();
    // The desktop entry sits at the root.
    let desktop_name = fs
        .files()
        .map(|n| norm(&n.fullpath))
        .find(|p| p.components().count() == 1 && p.to_string_lossy().ends_with(".desktop"));
    let mut icon_name = None;
    if let Some(name) = desktop_name {
        if let Some(bytes) = read_file(&name, 256 * 1024) {
            let text = String::from_utf8_lossy(&bytes).into_owned();
            icon_name = crate::desktop::field(&text, "Icon");
            info.desktop = Some((name.to_string_lossy().into_owned(), text));
        }
    }

    let mut try_icon = |p: &Path| -> bool {
        if let Some(bytes) = read_file(p, 4 << 20) {
            if let Some(ext) = image_ext(&bytes).filter(|e| *e != "jpg") {
                info.icon = Some((bytes, ext));
                return true;
            }
        }
        false
    };
    if !try_icon(Path::new(".DirIcon")) {
        if let Some(icon) = icon_name {
            let mut tried = false;
            for ext in ["svg", "png"] {
                if try_icon(Path::new(&format!("{icon}.{ext}"))) {
                    tried = true;
                    break;
                }
            }
            if !tried {
                // The icon theme inside, largest first.
                let mut found: Vec<PathBuf> = fs
                    .files()
                    .map(|n| norm(&n.fullpath))
                    .filter(|p| {
                        let s = p.to_string_lossy();
                        s.contains("icons/hicolor/")
                            && (s.ends_with(&format!("/{icon}.svg"))
                                || s.ends_with(&format!("/{icon}.png")))
                    })
                    .collect();
                found.sort_by_key(|p| std::cmp::Reverse(icon_rank(p)));
                for p in found {
                    if try_icon(&p) {
                        break;
                    }
                }
            }
        }
    }
    Ok(info)
}

fn icon_rank(p: &Path) -> u32 {
    let s = p.to_string_lossy();
    if s.contains("scalable") {
        return 10_000;
    }
    s.split('/')
        .find_map(|part| part.split_once('x').and_then(|(w, _)| w.parse::<u32>().ok()))
        .unwrap_or(0)
}

fn normalize(p: &Path) -> Option<PathBuf> {
    let mut out: Vec<String> = Vec::new();
    for c in p.components() {
        match c {
            std::path::Component::Normal(n) => out.push(n.to_string_lossy().into_owned()),
            std::path::Component::ParentDir => {
                out.pop()?;
            }
            _ => {}
        }
    }
    Some(out.iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_icons() {
        assert!(icon_rank(Path::new("usr/share/icons/hicolor/256x256/apps/a.png"))
            > icon_rank(Path::new("usr/share/icons/hicolor/48x48/apps/a.png")));
        assert!(icon_rank(Path::new("usr/share/icons/hicolor/scalable/apps/a.svg"))
            > icon_rank(Path::new("usr/share/icons/hicolor/512x512/apps/a.png")));
    }
}
