//! Starting and stopping the whole proxy (hosts, HTTPS bridge, ENet bridge)
//! from the console or the desktop window.

use anyhow::{Context, Result, anyhow};
use std::path::PathBuf;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use crate::config::Config;
use crate::state::Shared;
use crate::{cert, hosts, http, proxy};

pub enum StartError {
    /// The hosts file needs administrator rights.
    NeedAdmin,
    Other(anyhow::Error),
}

impl From<anyhow::Error> for StartError {
    fn from(e: anyhow::Error) -> Self {
        Self::Other(e)
    }
}

pub struct Engine {
    pub shared: Arc<Shared>,
    thread: Option<JoinHandle<Result<()>>>,
}

impl Engine {
    pub fn start(cfg: Config) -> Result<Self, StartError> {
        let cfg = Arc::new(cfg);
        let shared = Arc::new(Shared::default());
        // Fail here, not in the background thread, if the certificates are broken.
        cert::server_tls_config(&cert::ensure(&cfg.system.cert_dir)?)?;

        let guard = if cfg.system.manage_hosts {
            match hosts::install(&cfg.proxy.local_ip) {
                Ok(()) => {
                    tracing::info!("hosts file points Growtopia at {}", cfg.proxy.local_ip);
                    Some(hosts::HostsGuard)
                }
                Err(e) if permission_denied(&e) => return Err(StartError::NeedAdmin),
                Err(e) => return Err(StartError::Other(e)),
            }
        } else {
            tracing::info!("system.manage_hosts = false: point the Growtopia domains at {} yourself", cfg.proxy.local_ip);
            None
        };

        crate::report::start(&cfg.report);
        let thread = std::thread::Builder::new()
            .name("proxy".into())
            .spawn({
                let shared = shared.clone();
                move || {
                    let result = serve(cfg, shared);
                    if let Err(e) = &result {
                        tracing::error!("proxy stopped: {e:#}");
                    }
                    drop(guard); // restores the hosts file
                    result
                }
            })
            .context("cannot start the proxy thread")?;
        Ok(Self { shared, thread: Some(thread) })
    }

    /// True once the proxy has stopped on its own (port in use, crash...).
    pub fn is_finished(&self) -> bool {
        self.thread.as_ref().is_none_or(|t| t.is_finished())
    }

    /// Stops the proxy and waits for the hosts file to be restored.
    pub fn stop(mut self) -> Result<()> {
        self.shutdown()
    }

    fn shutdown(&mut self) -> Result<()> {
        self.shared.request_stop();
        match self.thread.take() {
            Some(t) => t.join().map_err(|_| anyhow!("proxy thread panicked"))?,
            None => Ok(()),
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

fn permission_denied(e: &anyhow::Error) -> bool {
    e.chain()
        .any(|c| c.downcast_ref::<std::io::Error>().is_some_and(|io| io.kind() == std::io::ErrorKind::PermissionDenied))
}

/// Runs both bridges until a stop is requested or one of them fails.
fn serve(cfg: Arc<Config>, shared: Arc<Shared>) -> Result<()> {
    let tls = cert::server_tls_config(&cert::ensure(&cfg.system.cert_dir)?)?;
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to start async runtime")?;
    rt.block_on(async {
        let mut enet = tokio::task::spawn_blocking({
            let (cfg, shared) = (cfg.clone(), shared.clone());
            move || proxy::run(cfg, shared)
        });
        let result = tokio::select! {
            r = http::serve(cfg.clone(), shared.clone(), tls) => r,
            r = &mut enet => return r.context("ENet thread panicked")?,
            () = stop_requested(&shared) => Ok(()),
        };
        shared.request_stop();
        let _ = enet.await;
        result
    })
}

async fn stop_requested(shared: &Shared) {
    while !shared.stopped() {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// `system.game_path`, or the default install location.
pub fn game_path(cfg: &Config) -> Option<PathBuf> {
    if !cfg.system.game_path.trim().is_empty() {
        return Some(PathBuf::from(cfg.system.game_path.trim()));
    }
    let base = std::env::var_os("LOCALAPPDATA")?;
    Some(PathBuf::from(base).join(r"Growtopia\Growtopia.exe"))
}

pub fn launch_game(cfg: &Config) -> Result<()> {
    let path = game_path(cfg).context("cannot find the Growtopia folder")?;
    if !path.exists() {
        anyhow::bail!("Growtopia not found at {} (set the path in Settings)", path.display());
    }
    let mut cmd = std::process::Command::new(&path);
    if let Some(dir) = path.parent() {
        cmd.current_dir(dir);
    }
    cmd.spawn().with_context(|| format!("cannot start {}", path.display()))?;
    tracing::info!("started {}", path.display());
    Ok(())
}
