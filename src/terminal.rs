//! Terminal mode: the proxy without the window, in the background. The
//! terminal only shows the log, like the window's Log page; everything is
//! done in game with the proxy commands.
//!
//! Started with `gtproxy run`, or by a plain double click when
//! `system.terminal = true` (Settings > Oyun > Terminal modu). Closing the
//! terminal or Ctrl+C stops the proxy and restores the hosts file.

use std::time::Duration;

use anyhow::{Result, bail};

use crate::config::Config;
use crate::engine::{self, Engine, StartError};

pub fn run(cfg: Config) -> Result<()> {
    let engine = match Engine::start(cfg.clone()) {
        Ok(e) => e,
        Err(StartError::NeedAdmin) => {
            if relaunch_as_admin().is_ok() {
                return Ok(());
            }
            bail!("the hosts file needs administrator rights: run this terminal as administrator")
        }
        Err(StartError::Other(e)) => return Err(e),
    };
    println!(
        "\nGTProxy v{} is running in the background. Use the proxy commands in game.\nClose this window or press Ctrl+C to stop (the hosts file is restored).\n",
        env!("CARGO_PKG_VERSION")
    );
    if cfg.system.launch_game {
        match engine::launch_game(&cfg) {
            Ok(()) => println!("Starting Growtopia..."),
            Err(e) => println!("Could not start Growtopia: {e:#}"),
        }
    }

    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    rt.block_on(async {
        let signal = shutdown_signal();
        tokio::pin!(signal);
        loop {
            tokio::select! {
                r = &mut signal => {
                    tracing::info!("{} received, shutting down", r?);
                    return Ok::<(), anyhow::Error>(());
                }
                () = tokio::time::sleep(Duration::from_millis(100)) => {
                    if engine.is_finished() {
                        return Ok(());
                    }
                }
            }
        }
    })?;
    println!("Stopping, restoring the hosts file...");
    engine.stop()
}

/// Starts this program again as administrator in terminal mode (Windows asks once).
#[cfg(windows)]
fn relaunch_as_admin() -> Result<()> {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "shell32")]
    unsafe extern "system" {
        fn ShellExecuteW(hwnd: isize, op: *const u16, file: *const u16, params: *const u16, dir: *const u16, show: i32) -> isize;
    }
    let wide = |s: &std::ffi::OsStr| s.encode_wide().chain(Some(0)).collect::<Vec<u16>>();
    let exe = std::env::current_exe()?;
    let dir = std::env::current_dir()?;
    let (op, file, params, dir) = (wide("runas".as_ref()), wide(exe.as_os_str()), wide("run".as_ref()), wide(dir.as_os_str()));
    const SW_SHOWNORMAL: i32 = 1;
    // SAFETY: all strings are NUL-terminated UTF-16 buffers alive for the call.
    let r = unsafe { ShellExecuteW(0, op.as_ptr(), file.as_ptr(), params.as_ptr(), dir.as_ptr(), SW_SHOWNORMAL) };
    if r > 32 {
        println!("Restarted as administrator in a new window.");
        Ok(())
    } else {
        bail!("Windows refused (code {r})")
    }
}

#[cfg(not(windows))]
fn relaunch_as_admin() -> Result<()> {
    bail!("run as root")
}

/// Ctrl+C, and on Windows also closing the console window, logoff and
/// shutdown: Windows gives a few seconds to clean up, enough to restore hosts.
#[cfg(windows)]
async fn shutdown_signal() -> std::io::Result<&'static str> {
    use tokio::signal::windows;
    let mut close = windows::ctrl_close()?;
    let mut brk = windows::ctrl_break()?;
    let mut logoff = windows::ctrl_logoff()?;
    let mut shutdown = windows::ctrl_shutdown()?;
    tokio::select! {
        r = tokio::signal::ctrl_c() => r.map(|_| "Ctrl+C"),
        _ = close.recv() => Ok("console window close"),
        _ = brk.recv() => Ok("Ctrl+Break"),
        _ = logoff.recv() => Ok("logoff"),
        _ = shutdown.recv() => Ok("system shutdown"),
    }
}

#[cfg(not(windows))]
async fn shutdown_signal() -> std::io::Result<&'static str> {
    tokio::signal::ctrl_c().await.map(|_| "Ctrl+C")
}
