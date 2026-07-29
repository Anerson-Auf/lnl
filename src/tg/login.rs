use color_eyre::{eyre::WrapErr, Report, Result};
use ferogram::{Client, SendCodeOutcome, SignInError};
use std::net::SocketAddr;
use std::sync::Arc;

use super::handlers::handle_update;
use super::history::seed_dialogues_from_folder;
use super::util::prompt;
use crate::api::{self, AppState};
use crate::config::{log_mtproxy, types::Telegram, CONFIG};

const SESSION_FILE: &str = "lnl.session";

fn map_connect_err(e: ferogram::QuickConnectError) -> Report {
    let msg = format!("{e}");
    if msg.contains("Firebase") {
        return color_eyre::eyre::eyre!(
            "не удалось подключиться через MTProxy (ниже исходная ошибка).\n\
            Сообщение про Firebase — это запасной обход, он тут ни при чём.\n\n\
            {msg}"
        );
    }
    if msg.contains("0x15") || msg.contains("FakeTLS") {
        return color_eyre::eyre::eyre!(
            "{msg}\n\nTLS alert 0x15: прокси не принял FakeTLS."
        );
    }
    color_eyre::eyre::eyre!("{msg}")
}

pub async fn init_telegram() -> Result<()> {
    let config = CONFIG.get().expect("config");
    let api_id: i32 = config
        .api_id
        .parse()
        .wrap_err("api_id должен быть числом")?;

    let mtproxy = config.mtproxy()?;

    let mut builder = Client::builder()
        .api_id(api_id)
        .api_hash(&config.api_hash)
        .session(SESSION_FILE)
        .catch_up(true);

    let socks_addr = config
        .tg_socks5
        .trim()
        .strip_prefix("socks5://")
        .unwrap_or(config.tg_socks5.trim());
    let socks_addr = socks_addr.trim();

    if !socks_addr.is_empty() {
        println!("SOCKS5 → {socks_addr}");
        builder = builder.socks5(socks_addr).resilient_connect(true);
    } else if let Some(ref mp) = mtproxy {
        log_mtproxy(mp);
        builder = builder.mtproxy(mp.clone()).resilient_connect(false);
    } else {
        println!("Прокси нет — прямое подключение к Telegram.");
        builder = builder.resilient_connect(true);
    }

    let (client, shutdown) = builder.connect().await.map_err(map_connect_err)?;
    ensure_authorized(&client).await?;

    let client = Arc::new(client);
    let telegram = Arc::new(Telegram {
        dialogues: dashmap::DashMap::new(),
    });

    let folder = &config.tg_folder;
    println!("Seeding folder «{folder}»…");
    seed_dialogues_from_folder(&client, &telegram, folder, 30).await?;
    println!("Loaded {} dialogues from «{folder}»", telegram.dialogues.len());

    let state = Arc::new(AppState::new(Arc::clone(&client), Arc::clone(&telegram)));

    let bind: SocketAddr = std::env::var("LNL_BIND")
        .unwrap_or_else(|_| "0.0.0.0:8080".into())
        .parse()
        .wrap_err("LNL_BIND")?;
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .wrap_err("не удалось открыть LNL_BIND")?;

    let api_state = (*state).clone();
    let api_task = tokio::spawn(async move {
        if let Err(e) = api::serve(api_state, listener, bind).await {
            eprintln!("API error: {e:#}");
        }
    });

    let mut stream = client.stream_updates();

    println!("Waiting for messages... (Ctrl+C to quit)");
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            upd = stream.next() => {
                let Some(upd) = upd else { break };
                let s = Arc::clone(&state);
                handle_update(s, upd).await;
            }
        }
    }

    println!("Saving session...");
    client
        .save_session()
        .await
        .map_err(|e| color_eyre::eyre::eyre!("{e}"))?;
    shutdown.cancel();
    api_task.abort();

    Ok(())
}

async fn ensure_authorized(client: &Client) -> Result<()> {
    if client
        .is_authorized()
        .await
        .map_err(|e| color_eyre::eyre::eyre!("{e}"))?
    {
        return Ok(());
    }

    println!("Signing in...");
    let phone = prompt("Enter phone number (international, +7...): ")?;
    let outcome = client
        .request_login_code(&phone)
        .await
        .map_err(|e| color_eyre::eyre::eyre!("{e}"))?;

    let token = match outcome {
        SendCodeOutcome::AlreadyAuthorized(_) => return Ok(()),
        SendCodeOutcome::CodeRequired(t) => t,
    };

    let code = prompt("Enter code: ")?;
    match client.sign_in(&token, &code).await {
        Ok(_) => {}
        Err(SignInError::PasswordRequired(pw)) => {
            let hint = pw.hint().unwrap_or("none");
            let password = prompt(&format!("Enter password (hint {hint}): "))?;
            client
                .check_password(*pw, password.as_bytes())
                .await
                .map_err(|e| color_eyre::eyre::eyre!("{e}"))?;
        }
        Err(e) => return Err(color_eyre::eyre::eyre!("{e}")),
    }

    client
        .save_session()
        .await
        .map_err(|e| color_eyre::eyre::eyre!("{e}"))?;
    println!("Signed in.");
    Ok(())
}
