//! Correct FakeTLS (`0xEE` MTProxy) handshake used by Telegram / mtg / mtprotoproxy.
//!
//! ferogram 0.6.4 shipped a broken ClientHello (no timestamp XOR, too short,
//! missing extensions) and never consumed the ServerHello — proxies answer with
//! TLS Alert `0x15` or domain-front HTTPS.

use std::time::{SystemTime, UNIX_EPOCH};

use hmac::{Hmac, Mac};
use sha2::Sha256;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::error::ConnectError;

type HmacSha256 = Hmac<Sha256>;

const DIGEST_LEN: usize = 32;
const DIGEST_POS: usize = 11;

/// Build a browser-like TLS 1.3 ClientHello record (~517 bytes) with the given
/// 32-byte random field and SNI `domain`.
pub fn build_client_hello(domain: &str, random_field: &[u8; 32], session_id: &[u8; 32]) -> Vec<u8> {
    let domain_bytes = domain.as_bytes();

    let cipher_suites: &[u8] = &[
        0x13, 0x01, 0x13, 0x02, 0x13, 0x03, 0xc0, 0x2b, 0xc0, 0x2f, 0xc0, 0x2c, 0xc0, 0x30, 0xcc,
        0xa9, 0xcc, 0xa8, 0xc0, 0x13, 0xc0, 0x14, 0x00, 0x9c, 0x00, 0x9d, 0x00, 0x2f, 0x00, 0x35,
    ];

    let mut sni_inner = Vec::new();
    sni_inner.push(0x00);
    sni_inner.extend_from_slice(&(domain_bytes.len() as u16).to_be_bytes());
    sni_inner.extend_from_slice(domain_bytes);
    let mut sni_list = Vec::new();
    sni_list.extend_from_slice(&(sni_inner.len() as u16).to_be_bytes());
    sni_list.extend_from_slice(&sni_inner);
    let mut sni_ext = Vec::new();
    sni_ext.extend_from_slice(&[0x00, 0x00]);
    sni_ext.extend_from_slice(&(sni_list.len() as u16).to_be_bytes());
    sni_ext.extend_from_slice(&sni_list);

    let ems_ext: &[u8] = &[0x00, 0x17, 0x00, 0x00];
    let reneg_ext: &[u8] = &[0xff, 0x01, 0x00, 0x01, 0x00];
    let sup_grp_ext: &[u8] = &[0x00, 0x0a, 0x00, 0x08, 0x00, 0x06, 0x00, 0x1d, 0x00, 0x17, 0x00, 0x18];
    let ec_pf_ext: &[u8] = &[0x00, 0x0b, 0x00, 0x02, 0x01, 0x00];
    let ticket_ext: &[u8] = &[0x00, 0x23, 0x00, 0x00];

    let alpn = b"\x02h2\x08http/1.1";
    let mut alpn_ext = Vec::new();
    alpn_ext.extend_from_slice(&[0x00, 0x10]);
    alpn_ext.extend_from_slice(&((alpn.len() + 2) as u16).to_be_bytes());
    alpn_ext.extend_from_slice(&(alpn.len() as u16).to_be_bytes());
    alpn_ext.extend_from_slice(alpn);

    let status_ext: &[u8] = &[0x00, 0x05, 0x00, 0x05, 0x01, 0x00, 0x00, 0x00, 0x00];

    let sig_algs: &[u8] = &[
        0x04, 0x03, 0x08, 0x04, 0x04, 0x01, 0x05, 0x03, 0x08, 0x05, 0x05, 0x01, 0x08, 0x06, 0x06,
        0x01, 0x02, 0x01,
    ];
    let mut sig_algs_ext = Vec::new();
    sig_algs_ext.extend_from_slice(&[0x00, 0x0d]);
    sig_algs_ext.extend_from_slice(&((sig_algs.len() + 2) as u16).to_be_bytes());
    sig_algs_ext.extend_from_slice(&(sig_algs.len() as u16).to_be_bytes());
    sig_algs_ext.extend_from_slice(sig_algs);

    let sct_ext: &[u8] = &[0x00, 0x12, 0x00, 0x00];

    let mut x25519_pub = [0u8; 32];
    ferogram_crypto::fill_random(&mut x25519_pub);
    let mut key_share_entry = Vec::new();
    key_share_entry.extend_from_slice(&[0x00, 0x1d]);
    key_share_entry.extend_from_slice(&(x25519_pub.len() as u16).to_be_bytes());
    key_share_entry.extend_from_slice(&x25519_pub);
    let mut key_share_list = Vec::new();
    key_share_list.extend_from_slice(&(key_share_entry.len() as u16).to_be_bytes());
    key_share_list.extend_from_slice(&key_share_entry);
    let mut key_share_ext = Vec::new();
    key_share_ext.extend_from_slice(&[0x00, 0x33]);
    key_share_ext.extend_from_slice(&(key_share_list.len() as u16).to_be_bytes());
    key_share_ext.extend_from_slice(&key_share_list);

    let psk_kem_ext: &[u8] = &[0x00, 0x2d, 0x00, 0x02, 0x01, 0x01];
    let sup_ver_ext: &[u8] = &[0x00, 0x2b, 0x00, 0x05, 0x04, 0x03, 0x04, 0x03, 0x03];
    let compress_cert_ext: &[u8] = &[0x00, 0x1b, 0x00, 0x03, 0x02, 0x00, 0x02];

    let mut extensions = Vec::new();
    extensions.extend_from_slice(&sni_ext);
    extensions.extend_from_slice(ems_ext);
    extensions.extend_from_slice(reneg_ext);
    extensions.extend_from_slice(sup_grp_ext);
    extensions.extend_from_slice(ec_pf_ext);
    extensions.extend_from_slice(ticket_ext);
    extensions.extend_from_slice(&alpn_ext);
    extensions.extend_from_slice(status_ext);
    extensions.extend_from_slice(&sig_algs_ext);
    extensions.extend_from_slice(sct_ext);
    extensions.extend_from_slice(&key_share_ext);
    extensions.extend_from_slice(psk_kem_ext);
    extensions.extend_from_slice(sup_ver_ext);
    extensions.extend_from_slice(compress_cert_ext);

    // Pad so total TCP record ≈ 517 bytes (what mtg / mtprotoproxy expect).
    let current_total = 5 + 4 + 2 + 32 + 1 + 32 + 2 + cipher_suites.len() + 2 + 2 + extensions.len();
    let pad_needed = 517usize.saturating_sub(current_total).saturating_sub(4);
    let mut padding_ext = Vec::new();
    padding_ext.extend_from_slice(&[0x00, 0x15]);
    padding_ext.extend_from_slice(&(pad_needed as u16).to_be_bytes());
    padding_ext.resize(4 + pad_needed, 0);
    extensions.extend_from_slice(&padding_ext);

    let mut body = Vec::new();
    body.extend_from_slice(&[0x03, 0x03]);
    body.extend_from_slice(random_field);
    body.push(session_id.len() as u8);
    body.extend_from_slice(session_id);
    body.extend_from_slice(&(cipher_suites.len() as u16).to_be_bytes());
    body.extend_from_slice(cipher_suites);
    body.extend_from_slice(&[0x01, 0x00]);
    body.extend_from_slice(&(extensions.len() as u16).to_be_bytes());
    body.extend_from_slice(&extensions);

    let mut handshake = Vec::new();
    handshake.push(0x01);
    let bl = body.len() as u32;
    handshake.push(((bl >> 16) & 0xff) as u8);
    handshake.push(((bl >> 8) & 0xff) as u8);
    handshake.push((bl & 0xff) as u8);
    handshake.extend_from_slice(&body);

    let mut record = Vec::new();
    record.push(0x16);
    record.extend_from_slice(&[0x03, 0x01]); // TLS 1.0 record version — required by mtprotoproxy
    record.extend_from_slice(&(handshake.len() as u16).to_be_bytes());
    record.extend_from_slice(&handshake);
    record
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC key");
    mac.update(data);
    mac.finalize().into_bytes().into()
}

/// Write FakeTLS ClientHello and return the 32-byte client digest placed in random.
pub async fn write_client_hello(
    stream: &mut TcpStream,
    secret: &[u8; 16],
    domain: &str,
) -> Result<[u8; 32], ConnectError> {
    let mut session_id = [0u8; 32];
    ferogram_crypto::fill_random(&mut session_id);

    let zero = [0u8; 32];
    let mut hello = build_client_hello(domain, &zero, &session_id);
    let mut digest = hmac_sha256(secret, &hello);

    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as u32;
    let ts_bytes = ts.to_le_bytes();
    for i in 0..4 {
        digest[DIGEST_LEN - 4 + i] ^= ts_bytes[i];
    }

    hello[DIGEST_POS..DIGEST_POS + DIGEST_LEN].copy_from_slice(&digest);
    stream.write_all(&hello).await?;
    Ok(digest)
}

async fn read_tls_record(stream: &mut TcpStream) -> Result<(u8, [u8; 2], Vec<u8>), ConnectError> {
    let mut hdr = [0u8; 5];
    stream.read_exact(&mut hdr).await?;
    let rtype = hdr[0];
    let version = [hdr[1], hdr[2]];
    let len = u16::from_be_bytes([hdr[3], hdr[4]]) as usize;
    if len > 1 << 16 {
        return Err(ConnectError::other(format!(
            "FakeTLS: implausible record length {len}"
        )));
    }
    let mut payload = vec![0u8; len];
    stream.read_exact(&mut payload).await?;
    Ok((rtype, version, payload))
}

/// Consume ServerHello + ChangeCipherSpec + first ApplicationData from the proxy.
///
/// Does not feed those bytes into the obfuscation cipher — they are FakeTLS
/// handshake only (mtg / mtprotoproxy).
pub async fn read_server_hello(
    stream: &mut TcpStream,
    client_digest: &[u8; 32],
    secret: &[u8; 16],
) -> Result<(), ConnectError> {
    let (rtype, version, payload) = read_tls_record(stream).await?;
    if rtype == 0x15 {
        return Err(ConnectError::other(
            "FakeTLS: TLS Alert 0x15 — proxy rejected ClientHello (secret/SNI/handshake mismatch)",
        ));
    }
    if rtype != 0x16 {
        return Err(ConnectError::other(format!(
            "FakeTLS: expected ServerHello handshake (0x16), got 0x{rtype:02x}"
        )));
    }
    if payload.first() != Some(&0x02) {
        return Err(ConnectError::other(
            "FakeTLS: first handshake record is not ServerHello",
        ));
    }

    let mut srv_stream = Vec::with_capacity(5 + payload.len());
    srv_stream.push(0x16);
    srv_stream.extend_from_slice(&version);
    srv_stream.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    srv_stream.extend_from_slice(&payload);

    // Drain CCS + AppData that follow (with a short idle timeout).
    let mut extra = Vec::new();
    loop {
        match tokio::time::timeout(std::time::Duration::from_millis(500), read_tls_record(stream))
            .await
        {
            Ok(Ok((t, ver, pay))) => {
                extra.push(t);
                extra.extend_from_slice(&ver);
                extra.extend_from_slice(&(pay.len() as u16).to_be_bytes());
                extra.extend_from_slice(&pay);
                // Typical FakeTLS response: SH + CCS + one AppData — stop after AppData.
                if t == 0x17 {
                    break;
                }
            }
            Ok(Err(e)) => return Err(e),
            Err(_) => break,
        }
    }

    let mut full = srv_stream;
    full.extend_from_slice(&extra);

    if full.len() >= DIGEST_POS + DIGEST_LEN {
        let server_digest = &full[DIGEST_POS..DIGEST_POS + DIGEST_LEN];
        let mut zeroed = full.clone();
        zeroed[DIGEST_POS..DIGEST_POS + DIGEST_LEN].fill(0);
        let mut mac_data = Vec::with_capacity(32 + zeroed.len());
        mac_data.extend_from_slice(client_digest);
        mac_data.extend_from_slice(&zeroed);
        let expected = hmac_sha256(secret, &mac_data);
        if expected.as_slice() != server_digest {
            tracing::warn!(
                "[ferogram::connect] FakeTLS: ServerHello digest mismatch — \
                 proxy may have domain-fronted; continuing anyway"
            );
        }
    }

    Ok(())
}

/// Perform full FakeTLS handshake (ClientHello → ServerHello/CCS/AppData).
pub async fn handshake(
    stream: &mut TcpStream,
    secret: &[u8; 16],
    domain: &str,
) -> Result<(), ConnectError> {
    let client_digest = write_client_hello(stream, secret, domain).await?;
    read_server_hello(stream, &client_digest, secret).await
}

/// Write a TLS record. `rtype` is 0x14 (CCS) or 0x17 (AppData).
pub async fn write_record(
    stream: &mut TcpStream,
    rtype: u8,
    payload: &[u8],
) -> Result<(), ConnectError> {
    const CHUNK: usize = 2878;
    for chunk in payload.chunks(CHUNK.max(1).min(payload.len().max(1))) {
        if payload.is_empty() {
            let mut hdr = [0u8; 5];
            hdr[0] = rtype;
            hdr[1] = 0x03;
            hdr[2] = 0x03;
            stream.write_all(&hdr).await?;
            break;
        }
        let mut rec = Vec::with_capacity(5 + chunk.len());
        rec.push(rtype);
        rec.extend_from_slice(&[0x03, 0x03]);
        rec.extend_from_slice(&(chunk.len() as u16).to_be_bytes());
        rec.extend_from_slice(chunk);
        stream.write_all(&rec).await?;
    }
    if payload.is_empty() && rtype == 0x14 {
        // ChangeCipherSpec is always 1 byte payload `\x01`.
    }
    Ok(())
}

/// Write ChangeCipherSpec (`14 03 03 00 01 01`) — required before first AppData.
pub async fn write_ccs(stream: &mut TcpStream) -> Result<(), ConnectError> {
    stream
        .write_all(&[0x14, 0x03, 0x03, 0x00, 0x01, 0x01])
        .await?;
    Ok(())
}

/// Write Application Data record(s) carrying already-obfuscated bytes.
pub async fn write_app_data(stream: &mut TcpStream, payload: &[u8]) -> Result<(), ConnectError> {
    const CHUNK: usize = 2878;
    for chunk in payload.chunks(CHUNK) {
        let mut rec = Vec::with_capacity(5 + chunk.len());
        rec.push(0x17);
        rec.extend_from_slice(&[0x03, 0x03]);
        rec.extend_from_slice(&(chunk.len() as u16).to_be_bytes());
        rec.extend_from_slice(chunk);
        stream.write_all(&rec).await?;
    }
    Ok(())
}

/// Read the next Application Data payload, skipping ChangeCipherSpec.
pub async fn read_app_data(stream: &mut TcpStream) -> Result<Vec<u8>, ConnectError> {
    loop {
        let (rtype, _, payload) = read_tls_record(stream).await?;
        match rtype {
            0x14 => continue,
            0x17 => return Ok(payload),
            0x15 => {
                return Err(ConnectError::other(
                    "FakeTLS: TLS Alert while reading application data",
                ));
            }
            other => {
                return Err(ConnectError::other(format!(
                    "FakeTLS: unexpected record type 0x{other:02x}"
                )));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_hello_is_padded_and_hmac_shaped() {
        let domain = "example.com";
        let session = [7u8; 32];
        let zero = [0u8; 32];
        let mut hello = build_client_hello(domain, &zero, &session);
        assert!(hello.len() >= 517, "len={}", hello.len());
        assert_eq!(&hello[0..3], &[0x16, 0x03, 0x01]);
        assert_eq!(&hello[DIGEST_POS..DIGEST_POS + DIGEST_LEN], &[0u8; 32]);

        let secret = [1u8; 16];
        let mut digest = hmac_sha256(&secret, &hello);
        let ts: u32 = 1_700_000_000;
        let tb = ts.to_le_bytes();
        for i in 0..4 {
            digest[28 + i] ^= tb[i];
        }
        hello[DIGEST_POS..DIGEST_POS + DIGEST_LEN].copy_from_slice(&digest);
        assert_ne!(&hello[DIGEST_POS..DIGEST_POS + 28], &[0u8; 28]);
    }
}
