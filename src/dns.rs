//! Minimal DNS A-record lookup over UDP.
//!
//! While the proxy runs, the hosts file points the Growtopia domains at us, so
//! the OS resolver would return 127.0.0.1. Asking a public DNS server directly
//! gives the real address.

use anyhow::{Context, Result, bail};
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::Duration;

pub fn resolve_a(name: &str, dns_server: &str) -> Result<Ipv4Addr> {
    let server: SocketAddr = dns_server.parse().with_context(|| format!("invalid dns_server {dns_server}"))?;
    let id: u16 = std::process::id() as u16 ^ 0x5a5a;
    let query = build_query(id, name)?;

    let sock = UdpSocket::bind("0.0.0.0:0")?;
    sock.set_read_timeout(Some(Duration::from_secs(3)))?;
    let mut buf = [0u8; 1500];
    for _ in 0..3 {
        sock.send_to(&query, server)?;
        let Ok((n, from)) = sock.recv_from(&mut buf) else { continue };
        if from != server {
            continue;
        }
        if let Some(ip) = parse_answer(&buf[..n], id) {
            return Ok(ip);
        }
        bail!("no A record for {name}");
    }
    bail!("DNS query for {name} to {server} timed out")
}

fn build_query(id: u16, name: &str) -> Result<Vec<u8>> {
    let mut q = Vec::with_capacity(32 + name.len());
    q.extend_from_slice(&id.to_be_bytes());
    q.extend_from_slice(&[0x01, 0x00]); // recursion desired
    q.extend_from_slice(&[0, 1, 0, 0, 0, 0, 0, 0]); // 1 question
    for label in name.trim_end_matches('.').split('.') {
        if label.is_empty() || label.len() > 63 {
            bail!("invalid domain {name}");
        }
        q.push(label.len() as u8);
        q.extend_from_slice(label.as_bytes());
    }
    q.push(0);
    q.extend_from_slice(&[0, 1, 0, 1]); // type A, class IN
    Ok(q)
}

fn parse_answer(r: &[u8], id: u16) -> Option<Ipv4Addr> {
    if r.len() < 12 || u16::from_be_bytes([r[0], r[1]]) != id || r[3] & 0x0f != 0 {
        return None;
    }
    let qd = u16::from_be_bytes([r[4], r[5]]);
    let an = u16::from_be_bytes([r[6], r[7]]);
    let mut pos = 12;
    for _ in 0..qd {
        pos = skip_name(r, pos)? + 4;
    }
    for _ in 0..an {
        pos = skip_name(r, pos)?;
        let rtype = u16::from_be_bytes([*r.get(pos)?, *r.get(pos + 1)?]);
        let rdlen = u16::from_be_bytes([*r.get(pos + 8)?, *r.get(pos + 9)?]) as usize;
        pos += 10;
        if rtype == 1 && rdlen == 4 {
            let d = r.get(pos..pos + 4)?;
            return Some(Ipv4Addr::new(d[0], d[1], d[2], d[3]));
        }
        pos += rdlen; // CNAME etc.
    }
    None
}

fn skip_name(r: &[u8], mut pos: usize) -> Option<usize> {
    loop {
        let len = *r.get(pos)?;
        if len == 0 {
            return Some(pos + 1);
        }
        if len & 0xc0 == 0xc0 {
            return Some(pos + 2); // compression pointer
        }
        pos += 1 + len as usize;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cname_then_a() {
        let id = 0x1234;
        let mut r = build_query(id, "www.growtopia1.com").unwrap();
        r[2] = 0x81;
        r[3] = 0x80;
        r[7] = 2; // two answers
        // CNAME answer, name = pointer to question
        r.extend_from_slice(&[0xc0, 0x0c, 0, 5, 0, 1, 0, 0, 0, 60, 0, 2, 0xc0, 0x0c]);
        // A answer
        r.extend_from_slice(&[0xc0, 0x0c, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4, 1, 2, 3, 4]);
        assert_eq!(parse_answer(&r, id), Some(Ipv4Addr::new(1, 2, 3, 4)));
        assert_eq!(parse_answer(&r, id + 1), None);
    }
}
