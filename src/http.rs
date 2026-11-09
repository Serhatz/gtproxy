//! HTTPS bridge for `/growtopia/server_data.php`.
//!
//! The client asks us (via the hosts file) where the game server is. We ask
//! the real server, remember its answer, and reply with our own ENet address.

use anyhow::{Context, Result, anyhow, bail};
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

use crate::config::Config;
use crate::dns;
use crate::protocol::TextPacket;
use crate::state::Shared;

const SERVER_DATA_PATH: &str = "/growtopia/server_data.php";

pub async fn serve(cfg: Arc<Config>, shared: Arc<Shared>, tls: Arc<rustls::ServerConfig>) -> Result<()> {
    let bind = format!("{}:{}", cfg.proxy.local_ip, cfg.proxy.http_port);
    let listener = TcpListener::bind(&bind)
        .await
        .with_context(|| format!("cannot bind HTTPS on {bind} (is another proxy or IIS using it?)"))?;
    let acceptor = TlsAcceptor::from(tls);
    tracing::info!("HTTPS server listening on {bind}");

    loop {
        let (tcp, peer) = listener.accept().await?;
        let acceptor = acceptor.clone();
        let cfg = cfg.clone();
        let shared = shared.clone();
        tokio::spawn(async move {
            let tls = match acceptor.accept(tcp).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("TLS handshake with {peer} failed: {e} (is the CA installed? `gtproxy cert install`)");
                    shared.note_tls_failure();
                    return;
                }
            };
            let svc = hyper::service::service_fn(move |req| handle(req, cfg.clone(), shared.clone()));
            if let Err(e) = hyper::server::conn::http1::Builder::new()
                .serve_connection(TokioIo::new(tls), svc)
                .await
            {
                tracing::debug!("HTTP connection from {peer} ended: {e}");
            }
        });
    }
}

async fn handle(
    req: Request<Incoming>,
    cfg: Arc<Config>,
    shared: Arc<Shared>,
) -> Result<Response<Full<Bytes>>, hyper::Error> {
    let method = req.method().clone();
    let path = req.uri().path().to_string();

    if method != Method::POST || path != SERVER_DATA_PATH {
        tracing::info!("{method} {path} -> 404");
        return Ok(reply(StatusCode::NOT_FOUND, "not found"));
    }

    let user_agent = req
        .headers()
        .get(hyper::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("UbiServices_SDK_2022.Release.9_PC64_ansi_static")
        .to_string();
    let body = req.into_body().collect().await?.to_bytes();
    tracing::debug!("server_data request body: {}", String::from_utf8_lossy(&body));

    match forward_server_data(&cfg, &user_agent, body).await {
        Ok((rewritten, upstream)) => {
            tracing::info!("server_data: real game server {upstream}, client redirected to proxy");
            shared.set_server_data_upstream(upstream);
            Ok(reply(StatusCode::OK, rewritten))
        }
        Err(e) => {
            tracing::error!("server_data failed: {e:#}");
            Ok(reply(StatusCode::BAD_GATEWAY, "proxy could not reach Growtopia"))
        }
    }
}

/// Fetches the real server_data.php and returns (rewritten body, real game server address).
async fn forward_server_data(cfg: &Config, user_agent: &str, body: Bytes) -> Result<(String, SocketAddr)> {
    let mut last_err = anyhow!("no server_data_hosts configured");
    for host in &cfg.upstream.server_data_hosts {
        match fetch_one(cfg, host, user_agent, body.clone()).await {
            Ok(text) => return rewrite(cfg, &text),
            Err(e) => {
                tracing::warn!("server_data via {host} failed: {e:#}");
                last_err = e;
            }
        }
    }
    Err(last_err)
}

async fn fetch_one(cfg: &Config, host: &str, user_agent: &str, body: Bytes) -> Result<String> {
    let ip = {
        let host = host.to_string();
        let dns_server = cfg.upstream.dns_server.clone();
        tokio::task::spawn_blocking(move || dns::resolve_a(&host, &dns_server)).await??
    };
    tracing::debug!("{host} resolves to {ip}");

    let client = reqwest::Client::builder()
        .resolve(host, SocketAddr::new(ip.into(), 443))
        .danger_accept_invalid_certs(!cfg.upstream.verify_tls)
        .timeout(Duration::from_secs(10))
        .build()?;
    let res = client
        .post(format!("https://{host}{SERVER_DATA_PATH}"))
        .header("User-Agent", user_agent)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Accept", "*/*")
        .body(body)
        .send()
        .await?;
    let status = res.status();
    let text = res.text().await?;
    if !status.is_success() {
        bail!("HTTP {status}");
    }
    Ok(text)
}

fn rewrite(cfg: &Config, text: &str) -> Result<(String, SocketAddr)> {
    let mut tp = TextPacket::parse(text);
    tracing::debug!("original server_data:\n{text}");
    // `#maint|...` is commented out and always present; only a bare `maint|` is active.
    if let Some(maint) = tp.get("maint") {
        tracing::warn!("server reports maintenance: {maint}");
    }
    let server = tp.get("server").context("server_data has no `server`")?.to_string();
    let port: u16 = tp.get("port").context("server_data has no `port`")?.parse()?;
    let upstream = (server.as_str(), port)
        .to_socket_addrs()?
        .find(SocketAddr::is_ipv4)
        .with_context(|| format!("cannot resolve game server {server}"))?;

    tp.set("server", cfg.proxy.local_ip.clone());
    tp.set("port", cfg.proxy.enet_port.to_string());
    Ok((tp.render(), upstream))
}

fn reply(status: StatusCode, body: impl Into<Bytes>) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header("Content-Type", "text/html")
        .body(Full::new(body.into()))
        .unwrap()
}
