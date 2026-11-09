// Release builds are a windowed app; command-line use re-attaches to the
// terminal it was started from (see `attach_console`).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod cert;
mod config;
mod dns;
mod embed;
mod engine;
mod gui;
mod gui_background;
mod hosts;
mod http;
mod items;
mod protocol;
mod proxy;
mod report;
mod state;
mod terminal;
mod ui_log;

use anyhow::{Result, bail};
use tracing_subscriber::EnvFilter;

use config::Config;

const LOG_PATH: &str = "gtproxy.log";

const USAGE: &str = "\
usage: gtproxy [command]

  (none)          open the window (or the terminal when system.terminal = true)
  gui             open the window
  run             terminal mode: the proxy in the background, this terminal shows the log
  cert install    trust the generated CA for the current Windows user
  cert uninstall  remove that CA again
  hosts remove    remove leftover gtproxy lines from the hosts file
  help            show this text";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let windowed = matches!(args.as_slice(), [] | ["--start"] | ["gui"]);
    if !windowed {
        attach_console();
    }
    use_project_dir();
    let cfg = Config::load_or_create()?;
    // A plain start opens the terminal instead of the window when chosen in Settings.
    let terminal = matches!(args.as_slice(), ["run"]) || (args.is_empty() && cfg.system.terminal);
    if terminal && windowed {
        attach_console();
    }
    init_logging(&cfg.log.level);
    // Panics go to gtproxy.log too, not only to the console.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!("panic: {info}");
        default_hook(info);
    }));

    match args.as_slice() {
        _ if terminal => terminal::run(cfg),
        [] | ["gui"] => gui::run(cfg, false),
        // Restarted elevated by the window's Start button.
        ["--start"] => gui::run(cfg, true),
        ["cert", "install"] => {
            cert::install(&cfg.system.cert_dir)?;
            println!("CA installed. Remove it later with `gtproxy cert uninstall`.");
            Ok(())
        }
        ["cert", "uninstall"] => cert::uninstall(),
        ["hosts", "remove"] => {
            let removed = hosts::uninstall()?;
            println!("{}", if removed { "hosts entries removed" } else { "nothing to remove" });
            Ok(())
        }
        ["help"] | ["-h"] | ["--help"] => {
            println!("{USAGE}");
            Ok(())
        }
        _ => bail!("unknown command\n\n{USAGE}"),
    }
}

/// config.toml, certs/, settings.json and the log live next to the program,
/// not in whatever folder it was started from. A build in `target/release`
/// uses the project folder above it, where the trusted certificate already is.
fn use_project_dir() {
    let Some(exe_dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(Into::into)) else { return };
    let exe_dir: std::path::PathBuf = exe_dir;
    let near = || exe_dir.ancestors().take(4);
    let dir = near()
        .find(|d| d.join("Cargo.toml").exists())
        .or_else(|| near().find(|d| d.join(config::CONFIG_PATH).exists()))
        .map(Into::into)
        .unwrap_or_else(|| exe_dir.clone());
    let _ = std::env::set_current_dir(dir);
}

/// Logs to the console, to `gtproxy.log` (overwritten each run) and to the window's Log tab.
fn init_logging(level: &str) {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level));
    let file = std::fs::File::create(LOG_PATH)
        .ok()
        .map(|f| tracing_subscriber::fmt::layer().with_target(false).with_ansi(false).with_writer(std::sync::Mutex::new(f)));
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().with_target(false))
        .with(file)
        .with(report::ErrorLayer)
        .with(tracing_subscriber::fmt::layer().with_target(false).with_ansi(false).with_writer(ui_log::writer))
        .init();
}

/// Lets a windowed release build print to the terminal it was started from,
/// or opens a console of its own (double click in terminal mode, elevated restart).
#[cfg(windows)]
fn attach_console() {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn AttachConsole(process: u32) -> i32;
        fn AllocConsole() -> i32;
        fn GetConsoleWindow() -> isize;
    }
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    // SAFETY: no pointers; both fail harmlessly when a console is already there.
    unsafe {
        if GetConsoleWindow() == 0 && AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
            AllocConsole();
        }
    }
}

#[cfg(not(windows))]
fn attach_console() {}
