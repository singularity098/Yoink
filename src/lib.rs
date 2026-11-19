//! Yoink: an ad-hoc package manager for apps, plugins and APKs published
//! on GitHub.

pub mod app;
pub mod appimage;
pub mod archive;
pub mod config;
pub mod desktop;
pub mod detect;
pub mod forge;
pub mod http;
mod icons;
pub mod library;
pub mod pattern;
pub mod plan;
pub mod secret;

slint::include_modules!();

/// Seconds since 1970.
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}


pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// `--probe`: what a repository offers, and what a pattern resolves to.
pub fn probe(args: &[String]) -> anyhow::Result<()> {
    use library::{Channel, Source};
    let repo = args.first().ok_or_else(|| anyhow::anyhow!("--probe needs OWNER/REPO"))?;
    let source = Source::parse(repo).ok_or_else(|| anyhow::anyhow!("not a repository: {repo}"))?;
    let channel = match args.get(1).map(String::as_str) {
        Some(a) if a.starts_with("actions") => Channel::Actions {
            workflow: String::new(),
            branch: a.split_once(':').map(|(_, b)| b.to_string()).unwrap_or_default(),
        },
        Some("pre") => Channel::Releases { prerelease: true },
        _ => Channel::Releases { prerelease: false },
    };
    let token = secret::load().map(|(t, _)| t);
    let forge = forge::forge(&source, token);
    let info = forge.info()?;
    println!("{} — {}", info.name, info.description);
    println!("default branch: {}; workflows: {}", info.default_branch, info.workflows.join(", "));
    let offer = forge.offer(&channel)?;
    println!("{} offers ({}):", channel.label(), offer.version);
    let best = detect::best_candidate(offer.files.iter().map(|f| f.name.as_str()));
    for (i, f) in offer.files.iter().enumerate() {
        println!(
            "  {} {:<50} {:>10}  {}",
            if Some(i) == best { "*" } else { " " },
            f.name,
            plan::human_size(f.size),
            detect::guess_kind(&f.name).label()
        );
    }
    if !offer.note.is_empty() {
        println!("  ({})", offer.note);
    }
    let pattern = match args.get(2) {
        Some(p) => p.clone(),
        None => match best {
            Some(i) => pattern::derive(&offer.files[i].name, Some(&offer.version)),
            None => return Ok(()),
        },
    };
    let r = forge.resolve(&channel, &pattern, false)?;
    println!("\n{pattern} resolves to {} ({}), {}", r.file_name, r.version, plan::human_size(r.size));
    println!("  key {}", r.key);
    if let Some(c) = &r.checksum {
        println!("  checksum from {}", c.url);
    }
    if !r.note.is_empty() {
        println!("  note: {}", r.note);
    }
    Ok(())
}

pub fn run() -> anyhow::Result<()> {
    app::run()
}

/// `--inspect FILE`: what Yoink sees in a downloaded file (no changes made).
pub fn inspect(path: &std::path::Path) -> anyhow::Result<()> {
    let t = archive::sniff(path);
    println!("{}: {t:?}", path.display());
    if t == archive::FileType::AppImage {
        let info = appimage::read(path)?;
        match &info.desktop {
            Some((name, text)) => println!("desktop entry {name}:\n{text}"),
            None => println!("no desktop entry"),
        }
        match &info.icon {
            Some((bytes, ext)) => println!("icon: {ext}, {} bytes", bytes.len()),
            None => println!("no icon"),
        }
    } else if t.is_archive() {
        let dir = std::env::temp_dir().join(format!("yoink-inspect-{}", std::process::id()));
        archive::extract_nested(path, &dir)?;
        let c = detect::scan(&dir);
        println!("{c:#?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
    Ok(())
}
