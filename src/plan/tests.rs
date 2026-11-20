use super::*;
use crate::library::{Channel, Formats, Installed, Source};
use std::os::unix::fs::PermissionsExt;

struct Env {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    dirs: Dirs,
}

fn env() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    let d = |n: &str| root.join("home").join(n).display().to_string();
    let dirs = Dirs {
        appimage: d("Applications"),
        bin: d("bin"),
        lib: d("lib"),
        share: d("share"),
        package: d("packages"),
        vst2: d("vst"),
        vst3: d("vst3"),
        lv2: d("lv2"),
        clap: d("clap"),
        apk: d("apk"),
        desktop: d("applications"),
        icons: d("icons"),
    };
    Env {
        _tmp: tmp,
        root,
        dirs,
    }
}

fn remote(name: &str, version: &str) -> Remote {
    Remote {
        key: format!("k-{version}"),
        version: version.into(),
        file_name: name.into(),
        size: 10,
        url: String::new(),
        accept: String::new(),
        needs_token: false,
        checksum: None,
        note: String::new(),
        web: String::new(),
        published: String::new(),
    }
}

fn app(kind: Kind) -> (App, Deployment) {
    let dep = Deployment::new(
        "1".into(),
        Channel::Releases { prerelease: false },
        kind,
        "*".into(),
    );
    let app = App {
        id: "o-tool".into(),
        name: "Tool".into(),
        source: Source::parse("o/tool").unwrap(),
        icon: None,
        icon_source: IconSource::None,
        description: String::new(),
        deployments: vec![dep.clone()],
    };
    (app, dep)
}

fn elf(exec: bool) -> Vec<u8> {
    let mut h = vec![0u8; 64];
    h[..4].copy_from_slice(b"\x7fELF");
    h[4] = 2;
    h[5] = 1;
    h[16] = if exec { 2 } else { 3 };
    h
}

fn write(path: &Path, data: &[u8], mode: u32) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, data).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}

/// A staged archive: `files` unpacked into a folder beside a dummy download.
fn staged(env: &Env, name: &str, files: &[(&str, Vec<u8>, u32)]) -> Staged {
    let dir = env.root.join("staging").join(name);
    let file = dir.join("download").join(name);
    write(&file, b"package", 0o644);
    let x = dir.join("unpacked");
    for (p, data, mode) in files {
        write(&x.join(p), data, *mode);
    }
    Staged {
        contents: detect::scan(&x),
        dir,
        file,
        ftype: FileType::TarGz,
        verified: None,
        unpacked: Some(x),
    }
}

fn ready(p: Prep) -> Task {
    match p {
        Prep::Ready(t) => *t,
        Prep::UpToDate => panic!("up to date"),
        Prep::NeedPick(p) => panic!("needs a pick: {p:?}"),
    }
}

fn stops(env: &Env) -> Vec<PathBuf> {
    crate::config::DIR_INFO
        .iter()
        .map(|d| env.dirs.path(d.0).unwrap())
        .collect()
}

#[test]
fn binary_prefix_layout() {
    let env = env();
    let lib = Library::default();
    let ctx = PlanContext {
        dirs: &env.dirs,
        library: &lib,
    };
    let (app, dep) = app(Kind::Binary);
    let s = staged(
        &env,
        "tool-1.0.tar.gz",
        &[
            ("tool-1.0/bin/tool", elf(true), 0o755),
            ("tool-1.0/lib/libtool.so", elf(false), 0o644),
            ("tool-1.0/share/tool/data.txt", b"d".to_vec(), 0o644),
            ("tool-1.0/etc/tool.conf", b"c".to_vec(), 0o644),
        ],
    );
    let t = ready(build(&ctx, &app, &dep, &remote("tool-1.0.tar.gz", "v1.0"), &s).unwrap());
    let h = env.root.join("home");
    assert_eq!(
        t.new_files,
        vec![
            h.join("packages/tool/tool-1.0.tar.gz"),
            h.join("bin/tool"),
            h.join("lib/libtool.so"),
            h.join("share/tool/data.txt"),
        ]
    );
    assert!(t
        .rows()
        .iter()
        .any(|r| r.verb == "Note" && r.text.contains("not deployed: etc")));
    let (placed, r) = apply(&t, &stops(&env));
    r.unwrap();
    assert_eq!(placed, t.new_files);
    let mode = std::fs::metadata(h.join("bin/tool")).unwrap().permissions().mode();
    assert_eq!(mode & 0o111, 0o111);
    assert_eq!(std::fs::read(h.join("share/tool/data.txt")).unwrap(), b"d");
}

#[test]
fn update_replaces_and_removes_old_files() {
    let env = env();
    let lib = Library::default();
    let ctx = PlanContext {
        dirs: &env.dirs,
        library: &lib,
    };
    let h = env.root.join("home");
    let (app, mut dep) = app(Kind::Binary);
    // Version 1: a different package name, an extra data file.
    write(&h.join("packages/tool/tool-1.0.tar.gz"), b"old", 0o644);
    write(&h.join("bin/tool"), b"old", 0o755);
    write(&h.join("share/tool/old/gone.txt"), b"old", 0o644);
    dep.installed = Some(Installed {
        key: "k-v1".into(),
        version: "v1".into(),
        files: vec![
            h.join("packages/tool/tool-1.0.tar.gz"),
            h.join("bin/tool"),
            h.join("share/tool/old/gone.txt"),
        ],
        when: 0,
    });
    let s = staged(
        &env,
        "tool-2.0.tar.gz",
        &[("tool-2.0/tool", elf(true), 0o755), ("tool-2.0/README", b"r".to_vec(), 0o644)],
    );
    let t = ready(build(&ctx, &app, &dep, &remote("tool-2.0.tar.gz", "v2"), &s).unwrap());
    let rows = t.rows();
    assert!(rows.iter().any(|r| r.verb == "Replace" && r.text.ends_with("bin/tool")));
    assert!(rows.iter().any(|r| r.verb == "Delete" && r.text.ends_with("tool-1.0.tar.gz")));
    assert!(rows.iter().any(|r| r.verb == "Delete" && r.text.ends_with("gone.txt")));
    assert_eq!(rows[0].detail, "v1 → v2");
    let (_, r) = apply(&t, &stops(&env));
    r.unwrap();
    assert!(!h.join("packages/tool/tool-1.0.tar.gz").exists());
    assert!(h.join("packages/tool/tool-2.0.tar.gz").exists());
    assert_eq!(std::fs::read(h.join("bin/tool")).unwrap(), elf(true));
    // Emptied folders are pruned, up to the share folder itself.
    assert!(!h.join("share/tool").exists());
    assert!(h.join("share").exists());
}

#[test]
fn several_executables_need_a_pick_once() {
    let env = env();
    let lib = Library::default();
    let ctx = PlanContext {
        dirs: &env.dirs,
        library: &lib,
    };
    let (app, mut dep) = app(Kind::Binary);
    let s = staged(
        &env,
        "t.zip",
        &[("t/tool", elf(true), 0o755), ("t/tool-helper", elf(true), 0o755)],
    );
    let r = remote("t.zip", "v1");
    let Prep::NeedPick(p) = build(&ctx, &app, &dep, &r, &s).unwrap() else {
        panic!("expected a pick");
    };
    assert_eq!(p.options, vec!["t/tool", "t/tool-helper"]);
    dep.inner = crate::pattern::derive(&p.options[1], Some(&p.version));
    dep.install_name = "helper".into();
    let t = ready(build(&ctx, &app, &dep, &r, &s).unwrap());
    assert!(t.new_files.contains(&env.root.join("home/bin/helper")));
}

#[test]
fn plugins_go_to_their_folders() {
    let env = env();
    let lib = Library::default();
    let ctx = PlanContext {
        dirs: &env.dirs,
        library: &lib,
    };
    let h = env.root.join("home");
    let (app, mut dep) = app(Kind::Plugin);
    let s = staged(
        &env,
        "synth.zip",
        &[
            ("Synth.vst3/Contents/x86_64-linux/Synth.so", elf(false), 0o755),
            ("Synth.lv2/manifest.ttl", b"m".to_vec(), 0o644),
            ("Synth.clap", elf(false), 0o755),
            ("vst2/Synth.so", elf(false), 0o755),
        ],
    );
    let r = remote("synth.zip", "v1");
    let Prep::NeedPick(p) = build(&ctx, &app, &dep, &r, &s).unwrap() else {
        panic!("expected the VST2 pick");
    };
    assert!(p.multi && p.vst2);
    assert_eq!(p.options, vec!["vst2/Synth.so"]);
    dep.vst2 = vec!["vst2/Synth.so".into()];
    // An older bundle in place is replaced whole.
    write(&h.join("vst3/Synth.vst3/stale.txt"), b"s", 0o644);
    dep.installed = Some(Installed {
        key: "k0".into(),
        version: "v0".into(),
        files: vec![h.join("vst3/Synth.vst3")],
        when: 0,
    });
    let t = ready(build(&ctx, &app, &dep, &r, &s).unwrap());
    for want in ["vst3/Synth.vst3", "lv2/Synth.lv2", "clap/Synth.clap", "vst/Synth.so", "packages/tool/synth.zip"] {
        assert!(t.new_files.contains(&h.join(want)), "{want}: {:?}", t.new_files);
    }
    let (_, res) = apply(&t, &stops(&env));
    res.unwrap();
    assert!(h.join("vst3/Synth.vst3/Contents/x86_64-linux/Synth.so").is_file());
    assert!(!h.join("vst3/Synth.vst3/stale.txt").exists());
    assert!(h.join("lv2/Synth.lv2/manifest.ttl").is_file());

    // VST2 turned off: no pick needed.
    let mut dep2 = dep.clone();
    dep2.vst2.clear();
    dep2.formats = Formats {
        vst2: false,
        ..Formats::default()
    };
    let t = ready(build(&ctx, &app, &dep2, &r, &s).unwrap());
    assert!(!t.new_files.contains(&h.join("vst/Synth.so")));
}

#[test]
fn appimage_from_a_zip_with_desktop_entry() {
    let env = env();
    let lib = Library::default();
    let ctx = PlanContext {
        dirs: &env.dirs,
        library: &lib,
    };
    let h = env.root.join("home");
    let (app, dep) = app(Kind::AppImage);
    assert!(dep.desktop);
    let mut ai = elf(true);
    ai[8] = b'A';
    ai[9] = b'I';
    ai[10] = 2;
    let s = staged(&env, "Tool-x86_64.AppImage.zip", &[("Tool-1.2-x86_64.AppImage", ai, 0o644)]);
    // A file already there that Yoink did not put there.
    write(&h.join("applications/tool.desktop"), b"mine", 0o644);
    let t = ready(build(&ctx, &app, &dep, &remote("Tool-x86_64.AppImage.zip", "v1.2"), &s).unwrap());
    let exe = h.join("Applications/Tool-1.2-x86_64.AppImage");
    assert_eq!(t.new_files[0], exe);
    let rows = t.rows();
    assert!(rows.iter().any(|r| r.verb == "Note" && r.text.contains("archive is then discarded")));
    let desktop = rows.iter().find(|r| r.text.ends_with("tool.desktop")).unwrap();
    assert_eq!(desktop.tone, 8, "an unmanaged file is flagged");
    let (_, res) = apply(&t, &stops(&env));
    res.unwrap();
    let mode = std::fs::metadata(&exe).unwrap().permissions().mode();
    assert_eq!(mode & 0o111, 0o111);
    let text = std::fs::read_to_string(h.join("applications/tool.desktop")).unwrap();
    assert!(text.contains(&format!("Exec={} %U", exe.display())), "{text}");
}

#[test]
fn removal_deletes_everything_recorded() {
    let env = env();
    let h = env.root.join("home");
    let (mut app, mut dep) = app(Kind::Apk);
    write(&h.join("apk/tool.apk"), b"a", 0o644);
    dep.installed = Some(Installed {
        key: "k".into(),
        version: "v".into(),
        files: vec![h.join("apk/tool.apk"), h.join("apk/missing.apk")],
        when: 0,
    });
    app.deployments = vec![dep];
    let t = removal(&app, None);
    assert_eq!(t.rows().iter().filter(|r| r.verb == "Delete").count(), 1);
    let (_, res) = apply(&t, &stops(&env));
    res.unwrap();
    assert!(!h.join("apk/tool.apk").exists());
    assert!(h.join("apk").exists());
}

#[test]
fn yoink_reinstalls_over_itself() {
    use std::io::Read;
    let env = env();
    let lib = Library::default();
    let ctx = PlanContext {
        dirs: &env.dirs,
        library: &lib,
    };
    let h = env.root.join("home");
    let (mut app, mut dep) = app(Kind::AppImage);
    app.source = Source::parse("singularity098/yoink").unwrap();
    assert!(app.is_yoink());
    let exe = h.join("Applications/Yoink-x86_64.AppImage");
    // The version installed (and running) now, recorded as this deployment's.
    write(&exe, b"old yoink", 0o755);
    dep.installed = Some(Installed {
        key: "k-v1".into(),
        version: "v1".into(),
        files: vec![exe.clone()],
        when: 0,
    });
    let mut ai = elf(true);
    ai[8] = b'A';
    ai[9] = b'I';
    ai[10] = 2;
    let dir = env.root.join("staging/yoink");
    let file = dir.join("download/Yoink-x86_64.AppImage");
    write(&file, &ai, 0o644);
    let s = Staged {
        contents: Contents::default(),
        dir,
        file,
        ftype: FileType::AppImage,
        verified: None,
        unpacked: None,
    };
    // The same version again (Force update), over the running AppImage.
    let t = ready(build(&ctx, &app, &dep, &remote("Yoink-x86_64.AppImage", "v1"), &s).unwrap());
    let rows = t.rows();
    assert!(rows.iter().any(|r| r.verb == "Note" && r.text.contains("this is Yoink itself")));
    assert!(!rows.iter().any(|r| r.tone == 8), "no warnings: {rows:?}");
    assert_eq!(t.new_files, vec![exe.clone()], "no desktop entry of its own");
    let mut running = std::fs::File::open(&exe).unwrap();
    let (_, res) = apply(&t, &stops(&env));
    res.unwrap();
    assert_eq!(std::fs::read(&exe).unwrap(), ai);
    // The running copy is untouched: the new one was renamed into place.
    let mut old = String::new();
    running.read_to_string(&mut old).unwrap();
    assert_eq!(old, "old yoink");
    assert!(!h.join("applications").exists());
}
