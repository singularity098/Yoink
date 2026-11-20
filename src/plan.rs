//! Turning updates into an itemised plan, and carrying it out.
//!
//! Preparing an update downloads the build into a staging folder in the
//! cache, checks it, unpacks it and looks inside, so the plan can say
//! exactly what will be copied where, what it replaces and what goes.
//! Nothing outside the cache changes until the plan is applied.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use anyhow::{anyhow, bail, Context};
use sha2::{Digest, Sha256};

use crate::archive::{self, FileType};
use crate::config::{cache_dir, tilde, Dirs};
use crate::detect::{self, Contents};
use crate::forge::{find_checksum, Forge, Remote};
use crate::library::{App, Deployment, IconSource, Kind, Library};
use crate::pattern::glob_match;

/// What a file about to be written replaces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Existing {
    Nothing,
    /// A file this deployment put there before.
    Ours,
    /// A file another app managed by Yoink put there.
    OtherApp(String),
    /// A file Yoink did not put there.
    Foreign,
}

#[derive(Clone, Debug)]
pub enum Action {
    /// The build downloaded into the staging folder.
    Downloaded {
        name: String,
        size: u64,
        verified: Option<bool>,
    },
    /// Copies a file or folder (a plugin bundle) into place.
    Install {
        src: PathBuf,
        dest: PathBuf,
        dir: bool,
        exec: bool,
        existing: Existing,
    },
    /// Keeps the package the files came from.
    KeepPackage {
        src: PathBuf,
        dest: PathBuf,
        existing: Existing,
    },
    /// Writes generated content (a desktop entry, an icon).
    Write {
        dest: PathBuf,
        contents: Vec<u8>,
        what: &'static str,
        existing: Existing,
    },
    /// Removes what the previous version put in place and this one does not.
    Delete { path: PathBuf, dir: bool },
    Note(String),
    Warn(String),
}

#[derive(Clone, Debug)]
pub struct Task {
    pub app_id: String,
    pub dep_id: String,
    pub app_name: String,
    pub label: String,
    pub from: Option<String>,
    pub to: String,
    pub remote: Option<Remote>,
    pub actions: Vec<Action>,
    /// Everything this deployment will have in place afterwards.
    pub new_files: Vec<PathBuf>,
    /// The icon found in the package, for the app's own icon.
    pub icon: Option<(Vec<u8>, &'static str, IconSource)>,
    /// Removes the app (or a deployment) rather than installing.
    pub removal: bool,
    pub staging: Option<PathBuf>,
}

/// One line of a plan, as shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// 0 heading, 1 download, 2 install, 3 replace, 4 delete, 5 keep,
    /// 6 write, 7 note, 8 warning.
    pub tone: i32,
    pub verb: String,
    pub text: String,
    pub detail: String,
}

impl Task {
    pub fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        let versions = match (&self.from, self.removal) {
            (_, true) => "remove".to_string(),
            (Some(f), _) if *f == self.to => format!("{} (again)", self.to),
            (Some(f), _) => format!("{f} → {}", self.to),
            (None, _) => format!("new: {}", self.to),
        };
        rows.push(Row {
            tone: 0,
            verb: String::new(),
            text: format!("{} · {}", self.app_name, self.label),
            detail: versions,
        });
        let existing_detail = |e: &Existing| match e {
            Existing::Nothing => String::new(),
            Existing::Ours => "replaces the previous version".into(),
            Existing::OtherApp(name) => format!("overwrites a file of {name}"),
            Existing::Foreign => "overwrites a file Yoink did not install".into(),
        };
        let replace_tone = |e: &Existing| match e {
            Existing::Nothing => 2,
            Existing::Ours => 3,
            _ => 8,
        };
        for a in &self.actions {
            let row = match a {
                Action::Downloaded {
                    name,
                    size,
                    verified,
                } => Row {
                    tone: 1,
                    verb: "Download".into(),
                    text: name.clone(),
                    detail: format!(
                        "{}{}",
                        human_size(*size),
                        match verified {
                            Some(true) => " · SHA-256 verified",
                            Some(false) => " · SHA-256 MISMATCH",
                            None => "",
                        }
                    ),
                },
                Action::Install {
                    dest,
                    dir,
                    existing,
                    ..
                } => Row {
                    tone: replace_tone(existing),
                    verb: if *existing == Existing::Nothing {
                        "Copy"
                    } else {
                        "Replace"
                    }
                    .into(),
                    text: format!("{}{}", tilde(dest), if *dir { "/" } else { "" }),
                    detail: existing_detail(existing),
                },
                Action::KeepPackage { dest, existing, .. } => Row {
                    tone: 5,
                    verb: "Keep".into(),
                    text: tilde(dest),
                    detail: if *existing == Existing::Nothing {
                        "the package".into()
                    } else {
                        format!("the package; {}", existing_detail(existing))
                    },
                },
                Action::Write {
                    dest,
                    what,
                    existing,
                    ..
                } => Row {
                    tone: if matches!(existing, Existing::Foreign | Existing::OtherApp(_)) {
                        8
                    } else {
                        6
                    },
                    verb: "Write".into(),
                    text: tilde(dest),
                    detail: if *existing == Existing::Nothing {
                        what.to_string()
                    } else {
                        format!("{what}; {}", existing_detail(existing))
                    },
                },
                Action::Delete { path, dir } => Row {
                    tone: 4,
                    verb: "Delete".into(),
                    text: format!("{}{}", tilde(path), if *dir { "/" } else { "" }),
                    detail: if self.removal {
                        String::new()
                    } else {
                        "from the previous version".into()
                    },
                },
                Action::Note(n) => Row {
                    tone: 7,
                    verb: "Note".into(),
                    text: n.clone(),
                    detail: String::new(),
                },
                Action::Warn(n) => Row {
                    tone: 8,
                    verb: "Warning".into(),
                    text: n.clone(),
                    detail: String::new(),
                },
            };
            rows.push(row);
        }
        rows
    }
}

pub fn human_size(n: u64) -> String {
    const K: f64 = 1024.0;
    let f = n as f64;
    if f >= K * K * K {
        format!("{:.1} GB", f / K / K / K)
    } else if f >= K * K {
        format!("{:.1} MB", f / K / K)
    } else if f >= K {
        format!("{:.0} KB", f / K)
    } else {
        format!("{n} B")
    }
}

/// Something only the user can decide before the plan can be made.
#[derive(Clone, Debug)]
pub struct Pick {
    pub app_id: String,
    pub dep_id: String,
    pub title: String,
    pub detail: String,
    pub options: Vec<String>,
    pub multi: bool,
    pub vst2: bool,
    pub version: String,
}

pub enum Prep {
    Ready(Box<Task>),
    UpToDate,
    NeedPick(Pick),
}

pub struct Staged {
    pub dir: PathBuf,
    pub file: PathBuf,
    pub ftype: FileType,
    pub verified: Option<bool>,
    /// Where the package was unpacked, if it is an archive.
    pub unpacked: Option<PathBuf>,
    pub contents: Contents,
}

fn staging_dir(remote: &Remote) -> PathBuf {
    let mut h = Sha256::new();
    h.update(remote.key.as_bytes());
    h.update(remote.url.as_bytes());
    let id = hex::encode(&h.finalize()[..8]);
    cache_dir().join("staging").join(id)
}

pub fn sha256_file(path: &Path) -> anyhow::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    std::io::copy(&mut f, &mut h)?;
    Ok(hex::encode(h.finalize()))
}

/// Downloads (unless already staged), verifies and unpacks a build.
pub fn stage(
    forge: &dyn Forge,
    remote: &Remote,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64, Option<u64>),
) -> anyhow::Result<Staged> {
    let dir = staging_dir(remote);
    let dl = dir.join("download");
    let name = sanitize_name(&remote.file_name);
    let file = dl.join(&name);
    let marker = dir.join("complete");
    let complete = std::fs::read_to_string(&marker)
        .map(|k| k == remote.key)
        .unwrap_or(false)
        && file.is_file();
    if !complete {
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dl)?;
        forge.download(&remote.url, &remote.accept, &file, cancel, progress)?;
        std::fs::write(&marker, &remote.key)?;
    }
    let verified = match &remote.checksum {
        None => None,
        Some(src) => match forge.fetch_small(&src.url, &src.accept) {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes);
                match find_checksum(&text, &remote.file_name) {
                    Some(want) => {
                        let got = sha256_file(&file)?;
                        if got != want {
                            let _ = std::fs::remove_dir_all(&dir);
                            bail!(
                                "{} does not match its published SHA-256 (got {got}, expected {want}); it was discarded",
                                remote.file_name
                            );
                        }
                        Some(true)
                    }
                    None => None,
                }
            }
            Err(e) => {
                log::warn!("could not fetch the checksum: {e:#}");
                None
            }
        },
    };
    let ftype = archive::sniff(&file);
    let mut unpacked = None;
    let mut contents = Contents::default();
    if ftype.is_archive() {
        let x = dir.join("unpacked");
        let _ = std::fs::remove_dir_all(&x);
        archive::extract_nested(&file, &x)
            .with_context(|| format!("unpacking {}", remote.file_name))?;
        contents = detect::scan(&x);
        unpacked = Some(x);
    }
    Ok(Staged {
        dir,
        file,
        ftype,
        verified,
        unpacked,
        contents,
    })
}

fn sanitize_name(name: &str) -> String {
    let n: String = name
        .chars()
        .map(|c| if c == '/' || c == '\0' { '_' } else { c })
        .collect();
    match n.as_str() {
        "" | "." | ".." => "download".into(),
        _ => n,
    }
}

/// Everything needed to work out a deployment's plan.
pub struct PlanContext<'a> {
    pub dirs: &'a Dirs,
    pub library: &'a Library,
}

impl PlanContext<'_> {
    fn existing(&self, dest: &Path, dep: &Deployment) -> Existing {
        if std::fs::symlink_metadata(dest).is_err() {
            return Existing::Nothing;
        }
        if dep
            .installed
            .as_ref()
            .is_some_and(|i| i.files.iter().any(|f| f == dest))
        {
            return Existing::Ours;
        }
        match self.library.owner_of(dest) {
            Some((app, _)) => Existing::OtherApp(app.name.clone()),
            None => Existing::Foreign,
        }
    }
}

enum Choice<'a> {
    One(&'a PathBuf),
    Pick(Vec<String>),
    None,
}

fn choose<'a>(cands: &'a [PathBuf], inner: &str) -> Choice<'a> {
    match cands.len() {
        0 => Choice::None,
        1 => Choice::One(&cands[0]),
        _ => {
            if !inner.is_empty() {
                if let Some(c) = cands
                    .iter()
                    .find(|c| glob_match(inner, &c.to_string_lossy()))
                {
                    return Choice::One(c);
                }
            }
            Choice::Pick(cands.iter().map(|c| c.to_string_lossy().into_owned()).collect())
        }
    }
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Works out the plan for one deployment and its staged build.
pub fn build(
    ctx: &PlanContext,
    app: &App,
    dep: &Deployment,
    remote: &Remote,
    staged: &Staged,
) -> anyhow::Result<Prep> {
    let mut actions = vec![Action::Downloaded {
        name: remote.file_name.clone(),
        size: std::fs::metadata(&staged.file).map(|m| m.len()).unwrap_or(remote.size),
        verified: staged.verified,
    }];
    if !remote.note.is_empty() {
        actions.push(Action::Note(remote.note.clone()));
    }
    let mut new_files: Vec<PathBuf> = Vec::new();
    let mut icon: Option<(Vec<u8>, &'static str, IconSource)> = None;
    let root = staged.unpacked.as_deref();
    let c = &staged.contents;
    let pick = |title: String, detail: String, options: Vec<String>, multi: bool, vst2: bool| -> anyhow::Result<Prep> {
        Ok(Prep::NeedPick(Pick {
            app_id: app.id.clone(),
            dep_id: dep.id.clone(),
            title,
            detail,
            options,
            multi,
            vst2,
            version: remote.version.clone(),
        }))
    };
    let install = |actions: &mut Vec<Action>,
                   new_files: &mut Vec<PathBuf>,
                   src: PathBuf,
                   dest: PathBuf,
                   dir: bool,
                   exec: bool| {
        let existing = ctx.existing(&dest, dep);
        actions.push(Action::Install {
            src,
            dest: dest.clone(),
            dir,
            exec,
            existing,
        });
        new_files.push(dest);
    };

    match dep.kind {
        Kind::AppImage | Kind::Apk => {
            let is_appimage = dep.kind == Kind::AppImage;
            let target_dir = ctx.dirs.path(if is_appimage { "appimage" } else { "apk" })?;
            let direct = if is_appimage {
                staged.ftype == FileType::AppImage
            } else {
                staged.ftype == FileType::Apk
            };
            let src = if direct {
                staged.file.clone()
            } else {
                let root = root.ok_or_else(|| {
                    anyhow!(
                        "{} is neither an {} nor an archive holding one",
                        remote.file_name,
                        dep.kind.label()
                    )
                })?;
                let list = if is_appimage { &c.appimages } else { &c.apks };
                match choose(list, &dep.inner) {
                    Choice::One(p) => {
                        actions.push(Action::Note(format!(
                            "{} is unpacked from {}; the archive is then discarded",
                            file_name(p),
                            remote.file_name
                        )));
                        root.join(p)
                    }
                    Choice::Pick(options) => {
                        return pick(
                            format!("Which {} is {}?", dep.kind.label(), app.name),
                            format!("{} holds several. Yoink remembers your choice.", remote.file_name),
                            options,
                            false,
                            false,
                        )
                    }
                    Choice::None => bail!("no {} inside {}", dep.kind.label(), remote.file_name),
                }
            };
            let dest = target_dir.join(file_name(&src));
            install(&mut actions, &mut new_files, src.clone(), dest.clone(), false, is_appimage);
            let yoink = is_appimage && app.is_yoink();
            if yoink {
                yoink_notes(&dest, dep.desktop, &mut actions);
            }
            if is_appimage {
                let info = crate::appimage::read(&src).unwrap_or_else(|e| {
                    log::info!("could not look inside {}: {e:#}", src.display());
                    Default::default()
                });
                if let Some((bytes, ext)) = &info.icon {
                    icon = Some((bytes.clone(), ext, IconSource::AppImage));
                }
                if dep.desktop && !yoink {
                    desktop_actions(ctx, app, dep, &dest, info.desktop, icon.as_ref(), &mut actions, &mut new_files)?;
                }
            }
        }
        Kind::Binary => {
            let bin = ctx.dirs.path("bin")?;
            let yoink = app.is_yoink();
            // Yoink's own binary is installed as `yoink`, whatever the
            // release calls it, unless asked otherwise.
            let exe_name = |file: String| {
                if !dep.install_name.trim().is_empty() {
                    dep.install_name.trim().to_string()
                } else if yoink {
                    "yoink".to_string()
                } else {
                    file
                }
            };
            if staged.ftype == FileType::Elf || (root.is_none() && staged.ftype == FileType::Other) {
                // A bare executable.
                let dest = bin.join(exe_name(file_name(&staged.file)));
                install(&mut actions, &mut new_files, staged.file.clone(), dest.clone(), false, true);
                if yoink {
                    yoink_notes(&dest, dep.desktop, &mut actions);
                } else if dep.desktop {
                    desktop_actions(ctx, app, dep, &dest, None, None, &mut actions, &mut new_files)?;
                }
            } else {
                let root = root.ok_or_else(|| anyhow!("{} is not an archive or executable", remote.file_name))?;
                keep_package(ctx, app, dep, staged, &mut actions, &mut new_files)?;
                let mut main_exe: Option<PathBuf> = None;
                let mut shipped_desktop = false;
                if let Some(prefix) = &c.prefix {
                    let lib = ctx.dirs.path("lib")?;
                    let share = ctx.dirs.path("share")?;
                    let mut ignored = Vec::new();
                    let top = root.join(prefix);
                    let mut entries: Vec<_> = std::fs::read_dir(&top)?.filter_map(|e| e.ok()).collect();
                    entries.sort_by_key(|e| e.file_name());
                    for e in entries {
                        let name = e.file_name().to_string_lossy().into_owned();
                        let target = match name.as_str() {
                            "bin" => &bin,
                            "lib" | "lib64" => &lib,
                            "share" => &share,
                            _ => {
                                ignored.push(name);
                                continue;
                            }
                        };
                        for f in walk_files(&e.path())? {
                            let rel = f.strip_prefix(e.path()).unwrap_or(&f).to_path_buf();
                            let mut dest = target.join(&rel);
                            if name == "bin" && rel.components().count() == 1 && !dep.install_name.trim().is_empty() {
                                // The main executable can take another name.
                                let is_main = match &main_exe {
                                    None => dep.inner.is_empty() || glob_match(&dep.inner, &f.strip_prefix(root).unwrap_or(&f).to_string_lossy()),
                                    Some(_) => false,
                                };
                                if is_main && detect::is_executable_file(&f) {
                                    dest = target.join(dep.install_name.trim());
                                }
                            }
                            if name == "bin" && main_exe.is_none() && detect::is_executable_file(&f) {
                                main_exe = Some(dest.clone());
                            }
                            if name == "share" && rel.starts_with("applications") && rel.extension().is_some_and(|x| x == "desktop") {
                                shipped_desktop = true;
                            }
                            let exec = detect::is_executable_file(&f);
                            install(&mut actions, &mut new_files, f, dest, false, exec);
                        }
                    }
                    if !ignored.is_empty() {
                        actions.push(Action::Note(format!(
                            "laid out as bin/lib/share; not deployed: {}",
                            ignored.join(", ")
                        )));
                    }
                } else {
                    let p = match choose(&c.executables, &dep.inner) {
                        Choice::One(p) => p,
                        Choice::Pick(options) => {
                            return pick(
                                format!("Which executable is {}?", app.name),
                                format!("{} holds several. It goes to your bin folder; Yoink remembers your choice.", remote.file_name),
                                options,
                                false,
                                false,
                            )
                        }
                        Choice::None => bail!("no executable inside {}", remote.file_name),
                    };
                    let dest = bin.join(exe_name(file_name(p)));
                    install(&mut actions, &mut new_files, root.join(p), dest.clone(), false, true);
                    if yoink {
                        yoink_notes(&dest, dep.desktop, &mut actions);
                    }
                    main_exe = Some(dest);
                    let others = c.executables.len() + c.shared_libs.len() - 1;
                    if others > 0 {
                        actions.push(Action::Note(format!(
                            "{others} other file{} in the package stay{} in the kept package",
                            if others == 1 { "" } else { "s" },
                            if others == 1 { "s" } else { "" }
                        )));
                    }
                }
                if dep.desktop && !yoink {
                    if shipped_desktop {
                        actions.push(Action::Note("the package brings its own desktop entry (in share/applications)".into()));
                    } else if let Some(exe) = &main_exe {
                        desktop_actions(ctx, app, dep, exe, None, None, &mut actions, &mut new_files)?;
                    }
                }
            }
        }
        Kind::Plugin => {
            let mut delivered = 0;
            let f = &dep.formats;
            if let Some(root) = root {
                keep_package(ctx, app, dep, staged, &mut actions, &mut new_files)?;
                let mut deliver = |list: &[PathBuf], dir_id: &str, on: bool, actions: &mut Vec<Action>, new_files: &mut Vec<PathBuf>| -> anyhow::Result<()> {
                    if !on || list.is_empty() {
                        return Ok(());
                    }
                    let target = ctx.dirs.path(dir_id)?;
                    for p in list {
                        let src = root.join(p);
                        let is_dir = src.is_dir();
                        install(actions, new_files, src, target.join(file_name(p)), is_dir, false);
                        delivered += 1;
                    }
                    Ok(())
                };
                deliver(&c.vst3, "vst3", f.vst3, &mut actions, &mut new_files)?;
                deliver(&c.lv2, "lv2", f.lv2, &mut actions, &mut new_files)?;
                deliver(&c.clap, "clap", f.clap, &mut actions, &mut new_files)?;
                if f.vst2 && !c.shared_libs.is_empty() {
                    if dep.vst2.is_empty() {
                        return pick(
                            format!("Which files are {}'s VST2 plugins?", app.name),
                            "VST2 plugins are plain .so files, so they cannot be told apart from other libraries. Tick the ones to put in your VST2 folder (none: no VST2). Yoink remembers your choice.".into(),
                            c.shared_libs.iter().map(|p| p.to_string_lossy().into_owned()).collect(),
                            true,
                            true,
                        );
                    }
                    let picked: Vec<&PathBuf> = c
                        .shared_libs
                        .iter()
                        .filter(|p| dep.vst2.iter().any(|g| glob_match(g, &p.to_string_lossy())))
                        .collect();
                    if picked.is_empty() {
                        actions.push(Action::Warn("none of the VST2 files picked before are in this version".into()));
                    } else {
                        let list: Vec<PathBuf> = picked.into_iter().cloned().collect();
                        deliver(&list, "vst2", true, &mut actions, &mut new_files)?;
                    }
                }
            } else {
                // A single plugin file.
                let name = remote.file_name.to_ascii_lowercase();
                let dir_id = if name.ends_with(".clap") {
                    "clap"
                } else if name.ends_with(".vst3") {
                    "vst3"
                } else {
                    "vst2"
                };
                let target = ctx.dirs.path(dir_id)?;
                install(&mut actions, &mut new_files, staged.file.clone(), target.join(file_name(&staged.file)), false, false);
                delivered += 1;
            }
            if delivered == 0 {
                bail!(
                    "no plugins found in {} for the formats chosen (VST3, LV2, CLAP{})",
                    remote.file_name,
                    if f.vst2 { ", VST2" } else { "" }
                );
            }
        }
    }

    // What the previous version had and this one does not.
    if let Some(inst) = &dep.installed {
        let keep: HashSet<&PathBuf> = new_files.iter().collect();
        for old in &inst.files {
            if keep.contains(old) {
                continue;
            }
            if let Ok(m) = std::fs::symlink_metadata(old) {
                actions.push(Action::Delete {
                    path: old.clone(),
                    dir: m.is_dir(),
                });
            }
        }
    }

    Ok(Prep::Ready(Box::new(Task {
        app_id: app.id.clone(),
        dep_id: dep.id.clone(),
        app_name: app.name.clone(),
        label: dep.label(),
        from: dep.installed.as_ref().map(|i| i.version.clone()),
        to: remote.version.clone(),
        remote: Some(remote.clone()),
        actions,
        new_files,
        icon,
        removal: false,
        staging: Some(staged.dir.clone()),
    })))
}

/// What Yoink runs from: its AppImage if it runs from one, else its own
/// executable (as it was started, even if it has since been replaced).
pub fn running_exe() -> Option<PathBuf> {
    std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| {
            let exe = std::env::current_exe().ok()?;
            let s = exe.to_string_lossy();
            Some(match s.strip_suffix(" (deleted)") {
                Some(s) => PathBuf::from(s),
                None => exe,
            })
        })
}

fn same_file(a: &Path, b: &Path) -> bool {
    a == b
        || match (a.canonicalize(), b.canonicalize()) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        }
}

/// Yoink updating itself: the executable it runs from is its own to
/// replace (the new one is renamed into place, so the running copy is
/// untouched until it restarts), and its menu entry is the one Options
/// manages.
fn yoink_notes(dest: &Path, desktop: bool, actions: &mut Vec<Action>) {
    let running = running_exe();
    let replaces_running = running.as_deref().is_some_and(|r| same_file(r, dest));
    if replaces_running {
        for a in actions.iter_mut() {
            if let Action::Install { dest: d, existing, .. } = a {
                if d == dest && *existing == Existing::Foreign {
                    *existing = Existing::Ours;
                }
            }
        }
    }
    actions.push(Action::Note(
        "this is Yoink itself: restart it afterwards to use the new version".into(),
    ));
    if let (Some(r), false) = (&running, replaces_running) {
        actions.push(Action::Note(format!(
            "Yoink now runs from {}; restarting runs the one installed here (the other is left alone)",
            tilde(r)
        )));
    }
    if desktop {
        actions.push(Action::Note(
            "no desktop entry is written for it: Yoink's own is set in Options, and follows wherever Yoink runs from"
                .into(),
        ));
    }
}

fn keep_package(
    ctx: &PlanContext,
    app: &App,
    dep: &Deployment,
    staged: &Staged,
    actions: &mut Vec<Action>,
    new_files: &mut Vec<PathBuf>,
) -> anyhow::Result<()> {
    let mut dest = ctx.dirs.path("package")?;
    if ctx.dirs.app_subdirs {
        dest.push(app.slug());
    }
    dest.push(file_name(&staged.file));
    let existing = ctx.existing(&dest, dep);
    actions.push(Action::KeepPackage {
        src: staged.file.clone(),
        dest: dest.clone(),
        existing,
    });
    new_files.push(dest);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn desktop_actions(
    ctx: &PlanContext,
    app: &App,
    dep: &Deployment,
    exe: &Path,
    shipped: Option<(String, String)>,
    package_icon: Option<&(Vec<u8>, &'static str, IconSource)>,
    actions: &mut Vec<Action>,
    new_files: &mut Vec<PathBuf>,
) -> anyhow::Result<()> {
    let desktop_dir = ctx.dirs.path("desktop")?;
    let icon_dir = ctx.dirs.path("icons")?;
    let slug = app.slug();
    // The icon: the package's own, else the app's (unless picked by hand,
    // the package's wins).
    let custom = app.icon_source == IconSource::Custom;
    let icon_bytes: Option<(Vec<u8>, String)> = match (package_icon, app.icon_path()) {
        (Some((b, e, _)), _) if !custom => Some((b.clone(), e.to_string())),
        (_, Some(p)) => std::fs::read(&p).ok().map(|b| {
            let e = p.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_else(|| "png".into());
            (b, e)
        }),
        (Some((b, e, _)), None) => Some((b.clone(), e.to_string())),
        _ => None,
    };
    let mut icon_path = None;
    if let Some((bytes, ext)) = icon_bytes {
        let dest = icon_dir.join(format!("{slug}.{ext}"));
        let existing = ctx.existing(&dest, dep);
        actions.push(Action::Write {
            dest: dest.clone(),
            contents: bytes,
            what: "icon",
            existing,
        });
        new_files.push(dest.clone());
        icon_path = Some(dest);
    }
    let (name, text) = match shipped {
        Some((name, text)) => {
            let text = crate::desktop::rewrite(&text, exe, icon_path.as_deref(), &app.id);
            (name, text)
        }
        None => (
            format!("{slug}.desktop"),
            crate::desktop::generate(&app.name, &app.description, exe, icon_path.as_deref(), &app.id),
        ),
    };
    let dest = desktop_dir.join(sanitize_name(&name));
    let existing = ctx.existing(&dest, dep);
    actions.push(Action::Write {
        dest: dest.clone(),
        contents: text.into_bytes(),
        what: "desktop entry",
        existing,
    });
    new_files.push(dest);
    Ok(())
}

fn walk_files(dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d)? {
            let e = e?;
            let t = e.file_type()?;
            if t.is_dir() {
                stack.push(e.path());
            } else {
                out.push(e.path());
            }
        }
    }
    out.sort();
    Ok(out)
}

/// A plan that removes everything a deployment (or every deployment of an
/// app, with `dep` None) put in place.
pub fn removal(app: &App, dep: Option<&str>) -> Task {
    let mut actions = Vec::new();
    for d in &app.deployments {
        if dep.is_some_and(|id| id != d.id) {
            continue;
        }
        if let Some(inst) = &d.installed {
            for f in &inst.files {
                if let Ok(m) = std::fs::symlink_metadata(f) {
                    actions.push(Action::Delete {
                        path: f.clone(),
                        dir: m.is_dir(),
                    });
                }
            }
        }
    }
    if actions.is_empty() {
        actions.push(Action::Note("nothing is installed".into()));
    }
    Task {
        app_id: app.id.clone(),
        dep_id: dep.unwrap_or("").to_string(),
        app_name: app.name.clone(),
        label: match dep.and_then(|id| app.deployments.iter().find(|d| d.id == id)) {
            Some(d) => d.label(),
            None => "all deployments".into(),
        },
        from: None,
        to: String::new(),
        remote: None,
        actions,
        new_files: Vec::new(),
        icon: None,
        removal: true,
        staging: None,
    }
}

// ------------------------------------------------------------- applying

/// Carries out a task's actions in order. Returns the files put in place,
/// also when it fails part way (so they can still be recorded and cleaned
/// up later).
pub fn apply(task: &Task, stop_dirs: &[PathBuf]) -> (Vec<PathBuf>, anyhow::Result<()>) {
    let mut placed = Vec::new();
    let result = (|| -> anyhow::Result<()> {
        for a in &task.actions {
            match a {
                Action::Install {
                    src,
                    dest,
                    dir,
                    exec,
                    ..
                } => {
                    place(src, dest, *dir, *exec)
                        .with_context(|| format!("copying to {}", tilde(dest)))?;
                    placed.push(dest.clone());
                }
                Action::KeepPackage { src, dest, .. } => {
                    place(src, dest, false, false)
                        .with_context(|| format!("keeping the package at {}", tilde(dest)))?;
                    placed.push(dest.clone());
                }
                Action::Write { dest, contents, .. } => {
                    crate::config::write_atomic(dest, contents)
                        .with_context(|| format!("writing {}", tilde(dest)))?;
                    placed.push(dest.clone());
                }
                Action::Delete { path, dir } => {
                    let r = if *dir {
                        std::fs::remove_dir_all(path)
                    } else {
                        std::fs::remove_file(path)
                    };
                    match r {
                        Ok(()) => prune(path, stop_dirs),
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                        Err(e) => return Err(anyhow!("deleting {}: {e}", tilde(path))),
                    }
                }
                Action::Downloaded { .. } | Action::Note(_) | Action::Warn(_) => {}
            }
        }
        Ok(())
    })();
    (placed, result)
}

/// Copies `src` to `dest` through a temporary name beside it, then swaps it
/// in, so `dest` is never half written. A folder replaces the old folder
/// whole (a plugin bundle).
fn place(src: &Path, dest: &Path, dir: bool, exec: bool) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let parent = dest.parent().ok_or_else(|| anyhow!("no parent folder"))?;
    std::fs::create_dir_all(parent)?;
    let name = file_name(dest);
    let tmp = parent.join(format!(".{name}.yoink-new"));
    let _ = remove_any(&tmp);
    let meta = std::fs::symlink_metadata(src)?;
    if meta.file_type().is_symlink() {
        let target = std::fs::read_link(src)?;
        std::os::unix::fs::symlink(target, &tmp)?;
    } else if dir || meta.is_dir() {
        copy_dir(src, &tmp)?;
    } else {
        std::fs::copy(src, &tmp)?;
        let mode = meta.permissions().mode() & 0o777;
        let mode = if exec { mode | 0o755 } else { mode | 0o644 };
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode))?;
    }
    match std::fs::symlink_metadata(dest) {
        Ok(m) if m.is_dir() && !m.file_type().is_symlink() => {
            // rename cannot replace a folder: move the old one aside first.
            let old = parent.join(format!(".{name}.yoink-old"));
            let _ = remove_any(&old);
            std::fs::rename(dest, &old)?;
            if let Err(e) = std::fs::rename(&tmp, dest) {
                let _ = std::fs::rename(&old, dest);
                return Err(e.into());
            }
            remove_any(&old)?;
        }
        _ => std::fs::rename(&tmp, dest)?,
    }
    Ok(())
}

fn remove_any(p: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(p) {
        Ok(m) if m.is_dir() => std::fs::remove_dir_all(p),
        Ok(_) => std::fs::remove_file(p),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

fn copy_dir(src: &Path, dest: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dest)?;
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let t = e.file_type()?;
        let to = dest.join(e.file_name());
        if t.is_symlink() {
            std::os::unix::fs::symlink(std::fs::read_link(e.path())?, &to)?;
        } else if t.is_dir() {
            copy_dir(&e.path(), &to)?;
        } else {
            std::fs::copy(e.path(), &to)?;
        }
    }
    Ok(())
}

/// Removes folders left empty above a deleted file, up to (not including)
/// the deployment folders.
fn prune(path: &Path, stop_dirs: &[PathBuf]) {
    let mut p = path.parent();
    for _ in 0..8 {
        let Some(dir) = p else { break };
        if stop_dirs.iter().any(|s| s == dir) || dir.parent().is_none() {
            break;
        }
        if std::fs::remove_dir(dir).is_err() {
            break;
        }
        p = dir.parent();
    }
}

/// Removes the staging folder of a task once applied (or abandoned).
pub fn discard_staging(task: &Task) {
    if let Some(dir) = &task.staging {
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// Removes staging folders left over from earlier runs.
pub fn clear_staging() {
    let _ = std::fs::remove_dir_all(cache_dir().join("staging"));
}

#[cfg(test)]
mod tests;
