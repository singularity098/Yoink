//! Telling what a downloaded file is, and unpacking archives: zip, tar
//! (plain, gzip, xz, zstd, bzip2) and 7z. Nothing is written outside the
//! folder unpacked into.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileType {
    Zip,
    Tar,
    TarGz,
    TarXz,
    TarZst,
    TarBz2,
    SevenZ,
    /// A gzip file that is not a tarball.
    Gz,
    AppImage,
    /// Another ELF executable or library.
    Elf,
    Apk,
    Other,
}

impl FileType {
    pub fn is_archive(self) -> bool {
        matches!(
            self,
            FileType::Zip
                | FileType::Tar
                | FileType::TarGz
                | FileType::TarXz
                | FileType::TarZst
                | FileType::TarBz2
                | FileType::SevenZ
                | FileType::Gz
        )
    }
}

fn head(path: &Path, n: usize) -> std::io::Result<Vec<u8>> {
    let mut f = File::open(path)?;
    let mut buf = vec![0u8; n];
    let mut got = 0;
    while got < n {
        let r = f.read(&mut buf[got..])?;
        if r == 0 {
            break;
        }
        got += r;
    }
    buf.truncate(got);
    Ok(buf)
}

/// What `path` is, from its first bytes (and its name, for APKs, which are
/// zip files, and for tarballs inside compression).
pub fn sniff(path: &Path) -> FileType {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let Ok(h) = head(path, 512) else {
        return FileType::Other;
    };
    if h.starts_with(b"\x7fELF") {
        // AppImages carry "AI" and their type at byte 8.
        if h.len() > 10 && &h[8..10] == b"AI" {
            return FileType::AppImage;
        }
        if name.ends_with(".appimage") {
            return FileType::AppImage;
        }
        return FileType::Elf;
    }
    if h.starts_with(b"PK\x03\x04") || h.starts_with(b"PK\x05\x06") {
        if name.ends_with(".apk") {
            return FileType::Apk;
        }
        return FileType::Zip;
    }
    if h.starts_with(b"7z\xbc\xaf\x27\x1c") {
        return FileType::SevenZ;
    }
    if h.len() > 262 && &h[257..262] == b"ustar" {
        return FileType::Tar;
    }
    let tarish = |n: &str| {
        n.contains(".tar.") || n.ends_with(".tgz") || n.ends_with(".txz") || n.ends_with(".tbz2")
    };
    if h.starts_with(&[0x1f, 0x8b]) {
        // Most gzip files here are tarballs; look inside to be sure.
        if tarish(&name) || inner_is_tar(path, FileType::TarGz) {
            return FileType::TarGz;
        }
        return FileType::Gz;
    }
    if h.starts_with(&[0xfd, b'7', b'z', b'X', b'Z', 0]) {
        return FileType::TarXz;
    }
    if h.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        return FileType::TarZst;
    }
    if h.starts_with(b"BZh") {
        return FileType::TarBz2;
    }
    FileType::Other
}

fn inner_is_tar(path: &Path, t: FileType) -> bool {
    let Ok(mut r) = decompressor(path, t) else {
        return false;
    };
    let mut buf = vec![0u8; 512];
    let mut got = 0;
    while got < 512 {
        match r.read(&mut buf[got..]) {
            Ok(0) | Err(_) => break,
            Ok(n) => got += n,
        }
    }
    got >= 262 && &buf[257..262] == b"ustar"
}

fn decompressor(path: &Path, t: FileType) -> anyhow::Result<Box<dyn Read>> {
    let f = BufReader::new(File::open(path)?);
    Ok(match t {
        FileType::Tar => Box::new(f),
        FileType::TarGz | FileType::Gz => Box::new(flate2::read::MultiGzDecoder::new(f)),
        FileType::TarXz => Box::new(XzReader::new(f)?),
        FileType::TarZst => Box::new(
            ruzstd::StreamingDecoder::new(f).map_err(|e| anyhow::anyhow!("zstd: {e}"))?,
        ),
        FileType::TarBz2 => Box::new(bzip2_rs::DecoderReader::new(f)),
        _ => bail!("not a compressed stream"),
    })
}

/// lzma-rs decodes xz from a reader into a writer in one go; this runs it
/// up front into memory. Packages are small enough for that.
struct XzReader {
    data: std::io::Cursor<Vec<u8>>,
}

impl XzReader {
    fn new(mut r: impl std::io::BufRead) -> anyhow::Result<XzReader> {
        let mut out = Vec::new();
        lzma_rs::xz_decompress(&mut r, &mut out).map_err(|e| anyhow::anyhow!("xz: {e:?}"))?;
        Ok(XzReader {
            data: std::io::Cursor::new(out),
        })
    }
}

impl Read for XzReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.data.read(buf)
    }
}

/// A relative path that stays inside the folder it is joined to.
fn safe_relative(p: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::Normal(n) => out.push(n),
            Component::CurDir => {}
            _ => return None,
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

/// Unpacks the archive at `path` into `dest`.
pub fn extract(path: &Path, t: FileType, dest: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dest)?;
    match t {
        FileType::Zip => extract_zip(path, dest),
        FileType::SevenZ => extract_7z(path, dest),
        FileType::Gz => {
            let stem = path
                .file_stem()
                .map(|s| s.to_os_string())
                .unwrap_or_else(|| "file".into());
            let mut r = decompressor(path, t)?;
            let out = dest.join(stem);
            let mut f = File::create(&out)?;
            std::io::copy(&mut r, &mut f)?;
            Ok(())
        }
        FileType::Tar | FileType::TarGz | FileType::TarXz | FileType::TarZst | FileType::TarBz2 => {
            let r = decompressor(path, t)?;
            let mut a = tar::Archive::new(r);
            a.set_preserve_permissions(true);
            a.set_overwrite(true);
            for entry in a.entries()? {
                let mut entry = entry?;
                // Devices and the like have no place in a package.
                let kind = entry.header().entry_type();
                if !(kind.is_file() || kind.is_dir() || kind.is_symlink() || kind.is_hard_link()) {
                    continue;
                }
                // unpack_in refuses paths leading outside `dest`.
                entry.unpack_in(dest)?;
            }
            Ok(())
        }
        _ => bail!("{} is not an archive", path.display()),
    }
}

fn extract_zip(path: &Path, dest: &Path) -> anyhow::Result<()> {
    let mut z = zip::ZipArchive::new(BufReader::new(File::open(path)?))
        .with_context(|| format!("reading {}", path.display()))?;
    for i in 0..z.len() {
        let mut f = z.by_index(i)?;
        let Some(rel) = f.enclosed_name().and_then(|p| safe_relative(&p)) else {
            log::warn!("skipping {} (outside the archive)", f.name());
            continue;
        };
        let out = dest.join(&rel);
        let mode = f.unix_mode();
        let is_link = mode.is_some_and(|m| m & 0o170000 == 0o120000);
        if f.is_dir() {
            std::fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if is_link {
            // A link inside the archive, pointing inside it.
            let mut target = String::new();
            f.read_to_string(&mut target)?;
            let resolved = rel.parent().unwrap_or(Path::new("")).join(&target);
            if target.starts_with('/') || normalize(&resolved).is_none() {
                log::warn!("skipping link {} -> {target}", f.name());
                continue;
            }
            let _ = std::fs::remove_file(&out);
            std::os::unix::fs::symlink(&target, &out)?;
            continue;
        }
        let mut w = File::create(&out)?;
        std::io::copy(&mut f, &mut w)?;
        if let Some(m) = mode {
            use std::os::unix::fs::PermissionsExt;
            let m = m & 0o777;
            if m != 0 {
                std::fs::set_permissions(&out, std::fs::Permissions::from_mode(m | 0o600))?;
            }
        }
    }
    Ok(())
}

/// `a/../b` → `b`; None if it climbs out.
fn normalize(p: &Path) -> Option<PathBuf> {
    let mut out: Vec<&std::ffi::OsStr> = Vec::new();
    for c in p.components() {
        match c {
            Component::Normal(n) => out.push(n),
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop()?;
            }
            _ => return None,
        }
    }
    Some(out.iter().collect())
}

fn extract_7z(path: &Path, dest: &Path) -> anyhow::Result<()> {
    let root = dest.to_path_buf();
    sevenz_rust2::decompress_file_with_extract_fn(path, dest, |entry, reader, _| {
        let Some(rel) = safe_relative(Path::new(entry.name())) else {
            return Ok(true);
        };
        let out = root.join(rel);
        sevenz_rust2::default_entry_extract_fn(entry, reader, &out)
    })
    .map_err(|e| anyhow::anyhow!("7z: {e}"))
}

/// Unpacks `path` into `dest`, then keeps unpacking while what came out is
/// a single archive (an Actions artifact's zip holding a tarball, say).
pub fn extract_nested(path: &Path, dest: &Path) -> anyhow::Result<()> {
    let t = sniff(path);
    extract(path, t, dest)?;
    for depth in 0..3 {
        let entries: Vec<_> = std::fs::read_dir(dest)?.filter_map(|e| e.ok()).collect();
        if entries.len() != 1 {
            break;
        }
        let only = entries[0].path();
        if !only.is_file() {
            break;
        }
        let t = sniff(&only);
        if !t.is_archive() {
            break;
        }
        let inner = dest.join(format!(".unpacked-{depth}"));
        extract(&only, t, &inner)?;
        std::fs::remove_file(&only)?;
        for e in std::fs::read_dir(&inner)? {
            let e = e?;
            std::fs::rename(e.path(), dest.join(e.file_name()))?;
        }
        std::fs::remove_dir(&inner)?;
    }
    Ok(())
}

/// The offset of the squashfs image inside an AppImage: the end of its ELF
/// runtime (section headers come last).
pub fn appimage_offset(path: &Path) -> anyhow::Result<u64> {
    let mut f = File::open(path)?;
    let mut h = [0u8; 64];
    f.read_exact(&mut h)?;
    anyhow::ensure!(&h[..4] == b"\x7fELF", "not an ELF file");
    let le = h[5] == 1;
    let u16_at = |o: usize| {
        let b = [h[o], h[o + 1]];
        if le {
            u16::from_le_bytes(b)
        } else {
            u16::from_be_bytes(b)
        }
    };
    let u32_at = |o: usize| {
        let b = [h[o], h[o + 1], h[o + 2], h[o + 3]];
        if le {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        }
    };
    let u64_at = |o: usize| {
        let mut b = [0u8; 8];
        b.copy_from_slice(&h[o..o + 8]);
        if le {
            u64::from_le_bytes(b)
        } else {
            u64::from_be_bytes(b)
        }
    };
    let offset = if h[4] == 2 {
        u64_at(0x28) + u64::from(u16_at(0x3a)) * u64::from(u16_at(0x3c))
    } else {
        u64::from(u32_at(0x20)) + u64::from(u16_at(0x2e)) * u64::from(u16_at(0x30))
    };
    let mut magic = [0u8; 4];
    f.seek(SeekFrom::Start(offset))?;
    if f.read_exact(&mut magic).is_ok() && &magic == b"hsqs" {
        return Ok(offset);
    }
    // Some runtimes differ: look for the magic in the first megabytes.
    f.seek(SeekFrom::Start(0))?;
    let mut buf = vec![0u8; 8 << 20];
    let n = f.read(&mut buf)?;
    buf.truncate(n);
    buf.windows(4)
        .position(|w| w == b"hsqs")
        .map(|p| p as u64)
        .ok_or_else(|| anyhow::anyhow!("no squashfs image found in the AppImage"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_zip(path: &Path, files: &[(&str, &[u8])]) {
        let f = File::create(path).unwrap();
        let mut z = zip::ZipWriter::new(f);
        for (name, data) in files {
            let opts = zip::write::SimpleFileOptions::default().unix_permissions(0o755);
            z.start_file(*name, opts).unwrap();
            z.write_all(data).unwrap();
        }
        z.finish().unwrap();
    }

    fn write_tar_gz(path: &Path, files: &[(&str, &[u8])]) {
        let f = File::create(path).unwrap();
        let gz = flate2::write::GzEncoder::new(f, flate2::Compression::fast());
        let mut t = tar::Builder::new(gz);
        for (name, data) in files {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o755);
            h.set_cksum();
            t.append_data(&mut h, name, *data).unwrap();
        }
        t.into_inner().unwrap().finish().unwrap();
    }

    #[test]
    fn sniffs_and_extracts() {
        let dir = tempfile::tempdir().unwrap();
        let z = dir.path().join("a.zip");
        write_zip(&z, &[("top/bin/tool", b"\x7fELF\x02\x01\x01"), ("top/README", b"hi")]);
        assert_eq!(sniff(&z), FileType::Zip);
        let out = dir.path().join("out");
        extract(&z, FileType::Zip, &out).unwrap();
        assert!(out.join("top/bin/tool").is_file());
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(out.join("top/bin/tool")).unwrap().permissions().mode();
        assert_eq!(mode & 0o111, 0o111);

        let apk = dir.path().join("app.apk");
        write_zip(&apk, &[("AndroidManifest.xml", b"x")]);
        assert_eq!(sniff(&apk), FileType::Apk);

        let tgz = dir.path().join("pkg.bin");
        write_tar_gz(&tgz, &[("x/y.txt", b"hello")]);
        assert_eq!(sniff(&tgz), FileType::TarGz);
    }

    #[test]
    fn nested_archives_unwrap() {
        let dir = tempfile::tempdir().unwrap();
        let tgz = dir.path().join("inner.tar.gz");
        write_tar_gz(&tgz, &[("plug.vst3/Contents/x86_64-linux/plug.so", b"\x7fELF")]);
        let data = std::fs::read(&tgz).unwrap();
        let z = dir.path().join("artifact.zip");
        write_zip(&z, &[("inner.tar.gz", &data)]);
        let out = dir.path().join("out");
        extract_nested(&z, &out).unwrap();
        assert!(out.join("plug.vst3/Contents/x86_64-linux/plug.so").is_file());
        assert!(!out.join("inner.tar.gz").exists());
    }

    #[test]
    fn zip_slip_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let z = dir.path().join("evil.zip");
        write_zip(&z, &[("../evil.txt", b"x"), ("ok.txt", b"y")]);
        let out = dir.path().join("out");
        extract(&z, FileType::Zip, &out).unwrap();
        assert!(!dir.path().join("evil.txt").exists());
        assert!(out.join("ok.txt").exists());
    }

    #[test]
    fn safe_paths() {
        assert!(safe_relative(Path::new("../x")).is_none());
        assert!(safe_relative(Path::new("/x")).is_none());
        assert_eq!(safe_relative(Path::new("./a/b")), Some(PathBuf::from("a/b")));
        assert_eq!(normalize(Path::new("a/../b")), Some(PathBuf::from("b")));
        assert_eq!(normalize(Path::new("../b")), None);
    }
}
