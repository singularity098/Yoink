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
                "Usage: yoink [--probe OWNER/REPO [releases|actions|pre] [PATTERN]]\n\n\
                 With no arguments, opens the window.\n\
                 --probe shows what a repository offers and, with a pattern, the build it\n\
                 would install (no downloads).\n\n\
                 Environment:\n  YOINK_HOME      keep settings, library and cache below this folder\n  \
                 YOINK_WAYLAND   1 for native Wayland, 0 for XWayland\n  \
                 SLINT_BACKEND   e.g. winit-software when no OpenGL is available"
            );
            Ok(())
        }
        Some("--probe") => yoink::probe(&args[1..]),
        Some("--inspect") if args.len() > 1 => yoink::inspect(std::path::Path::new(&args[1])),
        _ => yoink::run(),
    }
}
