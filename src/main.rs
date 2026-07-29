use color_eyre::{Result, eyre};

mod api;
mod config;
mod tg;

use config::{CONFIG, Config, harden_process_file_creation};
use tg::login::init_telegram;

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    dotenvy::dotenv().ok();

    harden_process_file_creation();
    let cfg = Config::from_env()?;
    cfg.prepare_session_files()?;
    CONFIG
        .set(cfg)
        .map_err(|_| eyre::eyre!("конфиг уже инициализирован"))?;

    init_telegram().await?;
    Ok(())
}
