//! What is in a package once unpacked (AppImages, APKs, executables,
//! plugin bundles, a bin/lib/share layout), and guessing a deployment's
//! kind and file from file names.

use std::path::{Path, PathBuf};

use crate::archive::{sniff, FileType};
use crate::library::Kind;

/// What an unpacked package holds, as paths relative to its root.
#[derive(Debug, Default, Clone)]
pub struct Contents {
    pub appimages: Vec<PathBuf>,
    pub apks: Vec<PathBuf>,
    /// ELF executables (not libraries).
    pub executables: Vec<PathBuf>,
    pub vst3: Vec<PathBuf>,
    pub lv2: Vec<PathBuf>,
    pub clap: Vec<PathBuf>,
    /// Shared libraries outside the bundles above: VST2 candidates.
    pub shared_libs: Vec<PathBuf>,
    /// A folder holding bin/ (and maybe lib/, share/): an install prefix.
    pub prefix: Option<PathBuf>,
    pub desktop_files: Vec<PathBuf>,
}

fn lower_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

fn is_shared_lib(name: &str) -> bool {
    name.ends_with(".so") || name.contains(".so.")
}

pub fn scan(root: &Path) -> Contents {
    let mut c = Contents::default();
    walk(root, root, &mut c, 0);
    for v in [
        &mut c.appimages,
        &mut c.apks,
        &mut c.executables,
        &mut c.vst3,
        &mut c.lv2,
        &mut c.clap,
        &mut c.shared_libs,
        &mut c.desktop_files,
    ] {
        v.sort();
    }
    c.prefix = find_prefix(root, &c);
    c
}

fn walk(root: &Path, dir: &Path, c: &mut Contents, depth: usize) {
    if depth > 12 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let path = e.path();
        let rel = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
        let name = lower_name(&path);
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_symlink() {
            continue;
        }
        if ft.is_dir() {
            // Plugin bundles are folders; what is inside them belongs to them.
            if name.ends_with(".vst3") {
                c.vst3.push(rel);
            } else if name.ends_with(".lv2") {
                c.lv2.push(rel);
            } else if name.ends_with(".clap") {
                c.clap.push(rel);
            } else if name.ends_with(".app") || name == "__macosx" {
                // macOS bundles: nothing for Linux in there.
            } else {
                walk(root, &path, c, depth + 1);
            }
            continue;
        }
        if name.ends_with(".desktop") {
            c.desktop_files.push(rel);
            continue;
        }
        if name.ends_with(".vst3") {
            c.vst3.push(rel);
            continue;
        }
        if name.ends_with(".clap") {
            c.clap.push(rel);
            continue;
        }
        match sniff(&path) {
            FileType::AppImage => c.appimages.push(rel),
            FileType::Apk => c.apks.push(rel),
            FileType::Elf => {
                if is_shared_lib(&name) || elf_is_library(&path) {
                    c.shared_libs.push(rel);
                } else {
                    c.executables.push(rel);
                }
            }
            _ => {}
        }
    }
}

/// ELF shared objects have type ET_DYN like position-independent
/// executables; executables have a program interpreter (PT_INTERP), plain
/// libraries do not.
fn elf_is_library(path: &Path) -> bool {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(path) else {
        return false;
    };
    let mut h = [0u8; 64];
    if f.read_exact(&mut h).is_err() {
        return false;
    }
    let le = h[5] == 1;
    let rd16 = |b: &[u8]| {
        if le {
            u16::from_le_bytes([b[0], b[1]])
        } else {
            u16::from_be_bytes([b[0], b[1]])
        }
    };
    let e_type = rd16(&h[16..18]);
    if e_type == 2 {
        return false; // ET_EXEC
    }
    if e_type != 3 {
        return true;
    }
    let is64 = h[4] == 2;
    let (phoff, phentsize, phnum) = if is64 {
        let mut b = [0u8; 8];
        b.copy_from_slice(&h[0x20..0x28]);
        let off = if le { u64::from_le_bytes(b) } else { u64::from_be_bytes(b) };
        (off, rd16(&h[0x36..0x38]), rd16(&h[0x38..0x3a]))
    } else {
        let b = [h[0x1c], h[0x1d], h[0x1e], h[0x1f]];
        let off = if le { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) };
        (u64::from(off), rd16(&h[0x2a..0x2c]), rd16(&h[0x2c..0x2e]))
    };
    for i in 0..u64::from(phnum.min(64)) {
        let mut p = [0u8; 4];
        if f.seek(SeekFrom::Start(phoff + i * u64::from(phentsize))).is_err()
            || f.read_exact(&mut p).is_err()
        {
            return false;
        }
        let p_type = if le { u32::from_le_bytes(p) } else { u32::from_be_bytes(p) };
        if p_type == 3 {
            return false; // PT_INTERP: an executable
        }
    }
    true
}

/// An executable file: one marked executable, or an ELF program.
pub fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let Ok(m) = std::fs::symlink_metadata(path) else {
        return false;
    };
    if !m.is_file() {
        return false;
    }
    if m.permissions().mode() & 0o111 != 0 {
        return true;
    }
    sniff(path) == FileType::Elf && !is_shared_lib(&lower_name(path)) && !elf_is_library(path)
}

/// The folder whose bin/ holds an executable, if the package is laid out
/// as an install prefix (bin/, lib/, share/).
fn find_prefix(root: &Path, c: &Contents) -> Option<PathBuf> {
    let mut best: Option<PathBuf> = None;
    for exe in &c.executables {
        let parent = exe.parent()?;
        if parent.file_name().map(|n| n == "bin").unwrap_or(false) {
            let prefix = parent.parent().unwrap_or(Path::new("")).to_path_buf();
            if prefix.components().count() > 2 {
                continue;
            }
            let has_more = ["lib", "lib64", "share"]
                .iter()
                .any(|d| root.join(&prefix).join(d).is_dir());
            if !has_more {
                continue;
            }
            if best
                .as_ref()
                .is_none_or(|b| prefix.components().count() < b.components().count())
            {
                best = Some(prefix);
            }
        }
    }
    best
}

/// A kind for a file offered by a release or run, from its name.
pub fn guess_kind(name: &str) -> Kind {
    let n = name.to_ascii_lowercase();
    if n.ends_with(".appimage") || n.contains("appimage") {
        Kind::AppImage
    } else if n.ends_with(".apk") || n.contains("android") || n.contains("apk") {
        Kind::Apk
    } else if ["vst", "lv2", "clap", "plugin", "plug-in"]
        .iter()
        .any(|w| n.contains(w))
    {
        Kind::Plugin
    } else {
        Kind::Binary
    }
}

/// How well a file suits this computer (higher is better; None: not at
/// all, like checksums or Windows installers).
pub fn suitability(name: &str) -> Option<i32> {
    let n = name.to_ascii_lowercase();
    let junk = [
        ".sha256", ".sha256sum", ".sha512", ".md5", ".sig", ".asc", ".minisig", ".sbom",
        ".json", ".txt", ".pem", ".crt", ".blockmap", ".yml", ".yaml", ".msi", ".exe", ".dmg",
        ".pkg", ".msix", ".appx", ".deb", ".rpm", ".snap", ".flatpak", ".nupkg",
    ];
    if junk.iter().any(|j| n.ends_with(j)) || n.contains("checksum") || n.contains("sha256sums") {
        return None;
    }
    if n == "source code" || n.contains("-src.") || n.contains("_src.") || n.contains("-source") || n.contains("_source") || n.starts_with("source") {
        return None;
    }
    let mut score = 0;
    let other_os = ["windows", "win64", "win32", "-win.", "_win.", "macos", "darwin", "osx", "-mac", "_mac", "apple", "freebsd", "ios"];
    if other_os.iter().any(|o| n.contains(o)) {
        score -= 50;
    }
    if n.contains("linux") {
        score += 20;
    }
    let arch = std::env::consts::ARCH;
    let (mine, others): (&[&str], &[&str]) = match arch {
        "aarch64" => (&["aarch64", "arm64"], &["x86_64", "amd64", "x64", "i686", "i386", "armv7", "armhf"]),
        _ => (&["x86_64", "amd64", "x64"], &["aarch64", "arm64", "armv7", "armhf", "i686", "i386", "riscv", "ppc64"]),
    };
    if mine.iter().any(|m| n.contains(m)) {
        score += 15;
    } else if others.iter().any(|o| n.contains(o)) {
        score -= 40;
    }
    if n.ends_with(".appimage") {
        score += 12;
    } else if n.ends_with(".apk") {
        score += 2;
    } else if n.ends_with(".tar.gz") || n.ends_with(".tgz") || n.ends_with(".tar.xz") || n.ends_with(".zip") || n.ends_with(".tar.zst") {
        score += 6;
    }
    if n.contains("musl") {
        score -= 2;
    }
    if n.contains("debug") || n.contains("symbols") || n.contains("dbg") {
        score -= 15;
    }
    Some(score)
}

/// The file to pick by default among those offered.
pub fn best_candidate<'a>(names: impl Iterator<Item = &'a str>) -> Option<usize> {
    names
        .enumerate()
        .filter_map(|(i, n)| suitability(n).map(|s| (i, s)))
        .max_by_key(|(i, s)| (*s, -(*i as i64)))
        .map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds() {
        assert_eq!(guess_kind("Chippy-x86_64.AppImage"), Kind::AppImage);
        assert_eq!(guess_kind("chippy-android.apk"), Kind::Apk);
        assert_eq!(guess_kind("Surge-XT-linux-vst3-lv2.zip"), Kind::Plugin);
        assert_eq!(guess_kind("chippy-linux-x86_64.tar.gz"), Kind::Binary);
    }

    #[test]
    fn picks_the_linux_file() {
        let names = [
            "app-windows-x64.zip",
            "app-macos.dmg",
            "app-linux-aarch64.tar.gz",
            "app-linux-x86_64.tar.gz",
            "SHA256SUMS.txt",
        ];
        let i = best_candidate(names.iter().copied()).unwrap();
        if std::env::consts::ARCH == "x86_64" {
            assert_eq!(names[i], "app-linux-x86_64.tar.gz");
        }
        assert_eq!(suitability("SHA256SUMS.txt"), None);
        assert_eq!(suitability("setup.exe"), None);
    }

    #[test]
    fn scans_a_package() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        let elf_exec = {
            // A minimal ELF64 header of type ET_EXEC.
            let mut h = vec![0u8; 64];
            h[..4].copy_from_slice(b"\x7fELF");
            h[4] = 2;
            h[5] = 1;
            h[16] = 2;
            h
        };
        let elf_lib = {
            let mut h = elf_exec.clone();
            h[16] = 3;
            h
        };
        std::fs::create_dir_all(r.join("pkg/bin")).unwrap();
        std::fs::create_dir_all(r.join("pkg/share/applications")).unwrap();
        std::fs::create_dir_all(r.join("pkg/Plug.vst3/Contents/x86_64-linux")).unwrap();
        std::fs::create_dir_all(r.join("pkg/Plug.lv2")).unwrap();
        std::fs::write(r.join("pkg/bin/tool"), &elf_exec).unwrap();
        std::fs::write(r.join("pkg/share/applications/tool.desktop"), "x").unwrap();
        std::fs::write(r.join("pkg/Plug.vst3/Contents/x86_64-linux/Plug.so"), &elf_lib).unwrap();
        std::fs::write(r.join("pkg/Plug.clap"), &elf_lib).unwrap();
        std::fs::write(r.join("pkg/PlugVST2.so"), &elf_lib).unwrap();
        let c = scan(r);
        assert_eq!(c.executables, vec![PathBuf::from("pkg/bin/tool")]);
        assert_eq!(c.vst3, vec![PathBuf::from("pkg/Plug.vst3")]);
        assert_eq!(c.lv2, vec![PathBuf::from("pkg/Plug.lv2")]);
        assert_eq!(c.clap, vec![PathBuf::from("pkg/Plug.clap")]);
        assert_eq!(c.shared_libs, vec![PathBuf::from("pkg/PlugVST2.so")]);
        assert_eq!(c.prefix, Some(PathBuf::from("pkg")));
        assert_eq!(c.desktop_files.len(), 1);
    }
}
