use color_eyre::{eyre, Result};
use color_eyre::eyre::WrapErr;
use std::env;

mod api;
mod config;
mod tg;

use config::{Config, CONFIG};
use tg::login::init_telegram;

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    dotenvy::dotenv().ok();

    let cfg = Config {
        api_id: env::var("api_id").wrap_err("нет api_id в env / .env")?,
        api_hash: env::var("api_hash").wrap_err("нет api_hash в env / .env")?,
        tg_proxy_link: env::var("TG_PROXY_LINK").unwrap_or_default(),
        tg_socks5: env::var("TG_SOCKS5").unwrap_or_default(),
        tg_folder: env::var("TG_FOLDER").wrap_err("нет TG_FOLDER — имя папки Telegram")?,
    };
    CONFIG
        .set(cfg)
        .map_err(|_| eyre::eyre!("конфиг уже инициализирован"))?;

    init_telegram().await?;
    Ok(())
}
