//! Hosts file redirection.
//!
//! Only lines between our own markers are ever added or removed; the rest of
//! the file is left exactly as it was. A copy is saved before the first edit.

use anyhow::{Context, Result};
use std::path::PathBuf;

const BEGIN: &str = "# >>> gtproxy (auto, removed on exit)";
const END: &str = "# <<< gtproxy";

pub const DOMAINS: &[&str] = &[
    "www.growtopia1.com",
    "www.growtopia2.com",
    "growtopia1.com",
    "growtopia2.com",
];

pub fn hosts_path() -> PathBuf {
    if cfg!(windows) {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        PathBuf::from(root).join(r"System32\drivers\etc\hosts")
    } else {
        PathBuf::from("/etc/hosts")
    }
}

/// Removes any previous gtproxy block (e.g. after a crash) and adds a fresh one.
pub fn install(ip: &str) -> Result<()> {
    let path = hosts_path();
    let original = std::fs::read_to_string(&path).with_context(|| format!("cannot read {}", path.display()))?;

    let backup = path.with_file_name("hosts.gtproxy.bak");
    if !backup.exists() {
        std::fs::write(&backup, &original).with_context(|| format!("cannot write {}", backup.display()))?;
    }

    let mut content = strip_block(&original);
    if !content.is_empty() && !content.ends_with('\n') {
        content.push_str(line_ending(&original));
    }
    let nl = line_ending(&original);
    content.push_str(BEGIN);
    content.push_str(nl);
    for d in DOMAINS {
        content.push_str(&format!("{ip} {d}{nl}"));
    }
    content.push_str(END);
    content.push_str(nl);

    write(&path, &content)
}

/// Removes our block. Safe to call when nothing was installed.
pub fn uninstall() -> Result<bool> {
    let path = hosts_path();
    let original = std::fs::read_to_string(&path).with_context(|| format!("cannot read {}", path.display()))?;
    let stripped = strip_block(&original);
    if stripped == original {
        return Ok(false);
    }
    write(&path, &stripped)?;
    Ok(true)
}

fn write(path: &PathBuf, content: &str) -> Result<()> {
    std::fs::write(path, content).with_context(|| {
        format!(
            "cannot write {} (run as administrator, or set system.manage_hosts = false and edit it yourself)",
            path.display()
        )
    })?;
    flush_dns();
    Ok(())
}

fn line_ending(s: &str) -> &'static str {
    if s.contains("\r\n") { "\r\n" } else { "\n" }
}

fn strip_block(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut inside = false;
    for line in s.split_inclusive('\n') {
        let t = line.trim_end();
        if t == BEGIN {
            inside = true;
            continue;
        }
        if inside {
            if t == END {
                inside = false;
            }
            continue;
        }
        out.push_str(line);
    }
    out
}

fn flush_dns() {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("ipconfig")
            .arg("/flushdns")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

/// Removes the hosts block when dropped, so a normal exit or a panic cleans up.
pub struct HostsGuard;

impl Drop for HostsGuard {
    fn drop(&mut self) {
        match uninstall() {
            Ok(true) => tracing::info!("hosts file restored"),
            Ok(false) => {}
            Err(e) => tracing::error!("failed to restore hosts file: {e:#}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_keeps_user_lines() {
        let user = "127.0.0.1 localhost\r\n10.0.0.1 nas\r\n";
        let with_block = format!("{user}{BEGIN}\r\n127.0.0.1 growtopia1.com\r\n{END}\r\n# after\r\n");
        assert_eq!(strip_block(&with_block), format!("{user}# after\r\n"));
        assert_eq!(strip_block(user), user);
    }
}
