pub mod types;
use color_eyre::{eyre::eyre, Result};
use ferogram::{parse_proxy_link, MtProxyConfig, TransportKind};

pub struct Config {
    pub api_id: String,
    pub api_hash: String,
    /// Полная `https://t.me/proxy?...` или пусто = без прокси (VPS).
    pub tg_proxy_link: String,
    /// `host:port` или `socks5://host:port` — выход через VPS без MTProxy-протокола.
    pub tg_socks5: String,
    /// Имя папки чатов Telegram (как в клиенте). Только эти чаты грузим в релей.
    pub tg_folder: String,
}

pub static CONFIG: std::sync::OnceLock<Config> = std::sync::OnceLock::new();

impl Config {
    pub fn mtproxy(&self) -> Result<Option<MtProxyConfig>> {
        let url = self.tg_proxy_link.trim();
        if url.is_empty() {
            return Ok(None);
        }

        let mut mp = parse_proxy_link(url)
            .ok_or_else(|| eyre!("TG_PROXY_LINK: не разобрать ссылку (нужен t.me/proxy?server=&port=&secret=)"))?;

        if let Ok(sni) = std::env::var("TG_PROXY_SNI") {
            if let TransportKind::FakeTls { ref mut domain, .. } = mp.transport {
                if !sni.trim().is_empty() {
                    *domain = sni.trim().to_string();
                }
            }
        }

        Ok(Some(mp))
    }
}

pub fn log_mtproxy(mp: &MtProxyConfig) {
    match &mp.transport {
        TransportKind::FakeTls { domain, .. } => {
            println!(
                "MTProxy: FakeTLS (ee) → {}:{} , SNI в ClientHello: {domain}",
                mp.host, mp.port
            );
        }
        TransportKind::PaddedIntermediate { .. } => {
            println!("MTProxy: PaddedIntermediate (dd) → {}:{}", mp.host, mp.port);
        }
        TransportKind::Obfuscated { .. } => {
            println!("MTProxy: Obfuscated → {}:{}", mp.host, mp.port);
        }
        other => println!("MTProxy: {:?} → {}:{}", other, mp.host, mp.port),
    }
}
