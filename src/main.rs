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
                 yoink --plasma-status | --install-plasma-env | --remove-plasma-env\n\n\
                 With no arguments, opens the window.\n\
                 --probe shows what a repository offers and, with a pattern, the build it\n\
                 would install (no downloads).\n\
                 --install-plasma-env writes ~/.config/plasma-workspace/env/yoink.sh so KDE\n\
                 Plasma looks in the desktop entries folder set in Options (from the next\n\
                 login); --remove-plasma-env deletes it; --plasma-status says where it stands.\n\n\
                 Environment:\n  YOINK_HOME      keep settings, library and cache below this folder\n  \
                 YOINK_WAYLAND   1 for native Wayland, 0 for XWayland\n  \
                 SLINT_BACKEND   e.g. winit-software when no OpenGL is available"
            );
            Ok(())
        }
        Some("--plasma-status") => {
            let dirs = yoink::config::Settings::load().dirs;
            println!("{}", yoink::plasma::state(&dirs).describe());
            Ok(())
        }
        Some("--install-plasma-env") => {
            let dirs = yoink::config::Settings::load().dirs;
            let path = yoink::plasma::install(&dirs)?;
            println!("Wrote {} (takes effect at the next login)", path.display());
            Ok(())
        }
        Some("--remove-plasma-env") => {
            yoink::plasma::remove()?;
            println!("Removed the Plasma script (takes effect at the next login)");
            Ok(())
        }
        Some("--probe") => yoink::probe(&args[1..]),
        Some("--inspect") if args.len() > 1 => yoink::inspect(std::path::Path::new(&args[1])),
        _ => yoink::run(),
    }
}
