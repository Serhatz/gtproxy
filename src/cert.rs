//! Local CA + server certificate for the HTTPS server_data.php bridge.
//!
//! Generated once into `cert_dir`. Installing the CA into the trust store is a
//! separate, explicit step (`gtproxy cert install`) and is never done
//! automatically; it goes into the *current user* store, not the machine one.

use anyhow::{Context, Result, bail};
use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose,
    date_time_ymd,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::hosts::DOMAINS;

pub const CA_COMMON_NAME: &str = "gtproxy local CA";

pub struct CertPaths {
    pub ca_cert: PathBuf,
    pub ca_key: PathBuf,
    pub server_cert: PathBuf,
    pub server_key: PathBuf,
}

impl CertPaths {
    pub fn new(dir: &str) -> Self {
        let d = Path::new(dir);
        Self {
            ca_cert: d.join("ca.pem"),
            ca_key: d.join("ca.key.pem"),
            server_cert: d.join("server.pem"),
            server_key: d.join("server.key.pem"),
        }
    }

    fn complete(&self) -> bool {
        [&self.ca_cert, &self.server_cert, &self.server_key].iter().all(|p| p.exists())
    }
}

/// Creates the CA and server certificate if they do not exist yet.
pub fn ensure(dir: &str) -> Result<CertPaths> {
    let paths = CertPaths::new(dir);
    if paths.complete() {
        return Ok(paths);
    }
    std::fs::create_dir_all(dir).with_context(|| format!("cannot create {dir}"))?;

    let year = current_year();

    let mut ca = CertificateParams::new(Vec::<String>::new())?;
    ca.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    ca.distinguished_name.push(DnType::CommonName, CA_COMMON_NAME);
    ca.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign, KeyUsagePurpose::DigitalSignature];
    ca.not_before = date_time_ymd(year - 1, 1, 1);
    ca.not_after = date_time_ymd(year + 10, 1, 1);
    let ca_key = KeyPair::generate()?;
    let ca_cert = ca.self_signed(&ca_key)?;

    let mut leaf = CertificateParams::new(DOMAINS.iter().map(|d| d.to_string()).collect::<Vec<_>>())?;
    leaf.distinguished_name.push(DnType::CommonName, DOMAINS[0]);
    leaf.key_usages = vec![KeyUsagePurpose::DigitalSignature, KeyUsagePurpose::KeyEncipherment];
    leaf.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    leaf.not_before = date_time_ymd(year - 1, 1, 1);
    leaf.not_after = date_time_ymd(year + 2, 1, 1);
    let leaf_key = KeyPair::generate()?;
    let leaf_cert = leaf.signed_by(&leaf_key, &ca_cert, &ca_key)?;

    std::fs::write(&paths.ca_cert, ca_cert.pem())?;
    std::fs::write(&paths.ca_key, ca_key.serialize_pem())?;
    std::fs::write(&paths.server_cert, format!("{}{}", leaf_cert.pem(), ca_cert.pem()))?;
    std::fs::write(&paths.server_key, leaf_key.serialize_pem())?;
    tracing::info!("generated new certificates in {dir}");
    Ok(paths)
}

pub fn server_tls_config(paths: &CertPaths) -> Result<Arc<rustls::ServerConfig>> {
    let certs = rustls_pemfile::certs(&mut std::fs::read(&paths.server_cert)?.as_slice())
        .collect::<Result<Vec<_>, _>>()
        .context("bad server.pem")?;
    let key = rustls_pemfile::private_key(&mut std::fs::read(&paths.server_key)?.as_slice())
        .context("bad server.key.pem")?
        .context("no private key in server.key.pem")?;

    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut cfg = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .with_no_client_auth()
        .with_single_cert(certs, key)?;
    cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(Arc::new(cfg))
}

/// Adds the CA to the current user's Root store (Windows shows a confirmation dialog).
pub fn install(dir: &str) -> Result<()> {
    let paths = ensure(dir)?;
    run_certutil(&["-user", "-addstore", "Root", &paths.ca_cert.to_string_lossy()])
}

pub fn uninstall() -> Result<()> {
    run_certutil(&["-user", "-delstore", "Root", CA_COMMON_NAME])
}

fn run_certutil(args: &[&str]) -> Result<()> {
    if !cfg!(windows) {
        bail!("certificate install is only automated on Windows; trust ca.pem manually");
    }
    let status = std::process::Command::new("certutil")
        .args(args)
        .status()
        .context("failed to start certutil")?;
    if !status.success() {
        bail!("certutil exited with {status}");
    }
    Ok(())
}

fn current_year() -> i32 {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    1970 + (secs / 31_556_952) as i32
}
