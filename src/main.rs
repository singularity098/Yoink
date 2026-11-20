//! Yoink on the command line, then the window.

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--version") => {
            println!("yoink {}", yoink::VERSION);
            Ok(())
        }
        Some("--help" | "-h") => {
            println!(
                "Usage: yoink [--probe OWNER/REPO [releases|pre|actions[:BRANCH]] [PATTERN]]\n       \
                 yoink --install-desktop | --remove-desktop\n\n\
                 With no arguments, opens the window.\n\
                 --probe shows what a repository offers and, with a pattern, the build it\n\
                 would install (no downloads).\n\
                 --install-desktop puts Yoink in the applications menu (a desktop entry\n\
                 and icon in the Desktop entries and Icons folders set in Options);\n\
                 --remove-desktop takes it out.\n\n\
                 Environment:\n  YOINK_HOME      keep settings, library and cache below this folder\n  \
                 YOINK_WAYLAND   1 for native Wayland, 0 for XWayland\n  \
                 SLINT_BACKEND   e.g. winit-software when no OpenGL is available"
            );
            Ok(())
        }
        Some("--install-desktop") => {
            let path = yoink::desktop::install_own_entry(&yoink::config::Settings::load().dirs)?;
            println!("Wrote {}", path.display());
            Ok(())
        }
        Some("--remove-desktop") => {
            yoink::desktop::remove_own_entry(&yoink::config::Settings::load().dirs)?;
            println!("Removed Yoink's desktop entry");
            Ok(())
        }
        Some("--probe") => yoink::probe(&args[1..]),
        Some("--inspect") if args.len() > 1 => yoink::inspect(std::path::Path::new(&args[1])),
        _ => yoink::run(),
    }
}
