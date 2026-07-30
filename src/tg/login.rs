use color_eyre::eyre::{WrapErr, eyre};
use color_eyre::{Report, Result};
use ferogram::{Client, MtProxyConfig, SendCodeOutcome, ShutdownToken, SignInError};
use std::io::IsTerminal;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

use super::accounts::{AccountManager, ManagedAccount};
use super::handlers::handle_update;
use super::util::prompt;
use crate::api::{self, AppState, SessionState};
use crate::config::{CONFIG, Config, SessionConfig, SessionId, log_mtproxy, secure_session_file};

const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

struct ConnectedAccount {
    config: SessionConfig,
    client: Arc<Client>,
    shutdown: ShutdownToken,
}

fn map_connect_err(error: ferogram::QuickConnectError) -> Report {
    let message = format!("{error}");
    if message.contains("Firebase") {
        return eyre!(
            "не удалось подключиться через MTProxy (ниже исходная ошибка).\n\
             Сообщение про Firebase — это запасной обход, он тут ни при чём.\n\n\
             {message}"
        );
    }
    if message.contains("0x15") || message.contains("FakeTLS") {
        return eyre!("{message}\n\nTLS alert 0x15: прокси не принял FakeTLS.");
    }
    eyre!("{message}")
}

pub async fn init_telegram() -> Result<()> {
    let config = CONFIG.get().expect("config");
    let mtproxy = config.mtproxy()?;
    let debug_ui = std::env::var("LNL_DEBUG_UI").ok().as_deref() == Some("1");

    let bind: SocketAddr = std::env::var("LNL_BIND")
        .unwrap_or_else(|_| "0.0.0.0:8080".into())
        .parse()
        .wrap_err("LNL_BIND")?;
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .wrap_err("не удалось открыть LNL_BIND")?;

    let admin_listener = if debug_ui {
        let admin_bind: SocketAddr = std::env::var("LNL_ADMIN_BIND")
            .unwrap_or_else(|_| "127.0.0.1:8081".to_string())
            .parse()
            .wrap_err("LNL_ADMIN_BIND")?;
        api::admin::validate_admin_bind(admin_bind).map_err(|error| eyre!(error))?;
        let token = std::env::var("LNL_ADMIN_TOKEN")
            .map_err(|_| eyre!("LNL_DEBUG_UI=1 требует LNL_ADMIN_TOKEN"))?;
        let token = api::admin::AdminToken::parse(token).map_err(|error| eyre!(error))?;
        let listener = tokio::net::TcpListener::bind(admin_bind)
            .await
            .wrap_err("не удалось открыть LNL_ADMIN_BIND")?;
        let actual_bind = listener
            .local_addr()
            .wrap_err("не удалось определить LNL_ADMIN_BIND")?;
        let access = api::admin::AdminAccess::new(token, format!("http://{actual_bind}"));
        Some((listener, actual_bind, access))
    } else {
        None
    };

    let mut connected = Vec::with_capacity(config.sessions.len());
    for session_config in &config.sessions {
        let account = match connect_account(config, session_config, mtproxy.as_ref()).await {
            Ok(account) => account,
            Err(error) => {
                let cleanup_errors = save_and_cancel_connected(&connected).await;
                return Err(with_cleanup(error, cleanup_errors));
            }
        };
        if !debug_ui
            && let Err(error) =
                ensure_authorized(&account.client, &session_config.id, &session_config.file).await
        {
            connected.push(account);
            let cleanup_errors = save_and_cancel_connected(&connected).await;
            return Err(with_cleanup(error, cleanup_errors));
        }
        connected.push(account);
    }

    let app_state = AppState::with_order(
        Vec::new(),
        config
            .sessions
            .iter()
            .map(|session| session.id.clone())
            .collect(),
        config.default_session.clone(),
    )
    .map_err(|error| eyre!(error))?;
    let update_stop = CancellationToken::new();
    let (worker_tx, mut worker_rx) = mpsc::unbounded_channel();
    let manager = AccountManager::new(
        connected
            .into_iter()
            .map(|account| {
                Arc::new(ManagedAccount::new(
                    account.config,
                    account.client,
                    account.shutdown,
                ))
            })
            .collect(),
        config.default_session.clone(),
        app_state.clone(),
        update_stop,
        worker_tx,
    )
    .map_err(|error| eyre!(error))?;
    if let Err(error) = manager.bootstrap().await {
        let cleanup_errors = manager.shutdown().await;
        return Err(with_cleanup(
            eyre!("Telegram account bootstrap failed: {}", error.message),
            cleanup_errors,
        ));
    }

    let api_stop = app_state.api_shutdown();
    let mut public_api_task = tokio::spawn(api::serve_public(app_state.clone(), listener, bind));
    let mut admin_api_task = admin_listener.map(|(listener, admin_bind, access)| {
        tokio::spawn(api::serve_admin(
            app_state.clone(),
            Arc::clone(&manager),
            access,
            listener,
            admin_bind,
        ))
    });

    let ready = manager
        .summaries()
        .await
        .into_iter()
        .filter(|account| account.status == "ready")
        .count();
    println!(
        "Waiting for messages in {ready}/{} session(s)... (Ctrl+C to quit)",
        config.sessions.len()
    );
    let mut public_finished = false;
    let mut admin_finished = false;
    let primary_error = tokio::select! {
        signal = tokio::signal::ctrl_c() => {
            signal.err().map(|error| eyre!("Ctrl+C handler failed: {error}"))
        }
        result = &mut public_api_task => {
            public_finished = true;
            Some(api_exit_error(result))
        }
        result = async {
            match admin_api_task.as_mut() {
                Some(task) => Some(task.await),
                None => std::future::pending().await,
            }
        } => {
            admin_finished = true;
            Some(api_exit_error(result.expect("admin task exists")))
        }
        result = worker_rx.recv() => {
            Some(worker_exit_error(result))
        }
    };

    api_stop.cancel();
    let mut cleanup_errors = Vec::new();
    if !public_finished {
        cleanup_errors.extend(stop_api(&mut public_api_task).await);
    }
    if !admin_finished && let Some(task) = admin_api_task.as_mut() {
        cleanup_errors.extend(stop_api(task).await);
    }
    cleanup_errors.extend(manager.shutdown().await);

    match primary_error {
        Some(error) => Err(with_cleanup(error, cleanup_errors)),
        None if cleanup_errors.is_empty() => Ok(()),
        None => Err(eyre!(
            "ошибки при остановке:\n{}",
            cleanup_errors.join("\n")
        )),
    }
}

async fn connect_account(
    config: &Config,
    session_config: &SessionConfig,
    mtproxy: Option<&MtProxyConfig>,
) -> Result<ConnectedAccount> {
    println!(
        "[session:{}] file: {}",
        session_config.id,
        session_config.file.display()
    );
    let mut builder = Client::builder()
        .api_id(config.api_id)
        .api_hash(&config.api_hash)
        .session(&session_config.file)
        .catch_up(true);

    let socks_addr = config
        .tg_socks5
        .trim()
        .strip_prefix("socks5://")
        .unwrap_or(config.tg_socks5.trim())
        .trim();
    if !socks_addr.is_empty() {
        println!("[session:{}] SOCKS5 → {socks_addr}", session_config.id);
        builder = builder.socks5(socks_addr).resilient_connect(true);
    } else if let Some(mtproxy) = mtproxy {
        print!("[session:{}] ", session_config.id);
        log_mtproxy(mtproxy);
        builder = builder.mtproxy(mtproxy.clone()).resilient_connect(false);
    } else {
        println!(
            "[session:{}] Прокси нет — прямое подключение к Telegram.",
            session_config.id
        );
        builder = builder.resilient_connect(true);
    }

    let (client, shutdown) = builder
        .connect()
        .await
        .map_err(map_connect_err)
        .wrap_err_with(|| format!("session «{}»: connect", session_config.id))?;
    Ok(ConnectedAccount {
        config: session_config.clone(),
        client: Arc::new(client),
        shutdown,
    })
}

pub(crate) async fn run_update_worker(
    state: Arc<SessionState>,
    stop: CancellationToken,
) -> Result<SessionId> {
    let session_id = state.id().clone();
    let mut updates = state.client.stream_updates();

    loop {
        tokio::select! {
            biased;
            _ = stop.cancelled() => return Ok(session_id),
            update = updates.next() => {
                let Some(update) = update else {
                    return Err(eyre!(
                        "Telegram update stream закрыт: session «{session_id}»"
                    ));
                };
                handle_update(&state, update).await;
            }
        }
    }
}

async fn ensure_authorized(
    client: &Client,
    session_id: &SessionId,
    session_file: &Path,
) -> Result<()> {
    if client
        .is_authorized()
        .await
        .map_err(|error| eyre!("{error}"))?
    {
        return Ok(());
    }
    if !std::io::stdin().is_terminal() {
        return Err(eyre!(
            "session «{session_id}» не авторизована ({}); \
             запусти lnl один раз в терминале или установи готовый session-файл",
            session_file.display()
        ));
    }

    println!("[session:{session_id}] Signing in...");
    let phone = prompt(&format!(
        "[session:{session_id}] Enter phone number (international, +7...): "
    ))?;
    let outcome = client
        .request_login_code(&phone)
        .await
        .map_err(|error| eyre!("{error}"))?;
    let token = match outcome {
        SendCodeOutcome::AlreadyAuthorized(_) => return Ok(()),
        SendCodeOutcome::CodeRequired(token) => token,
    };

    let code = rpassword::prompt_password(format!("[session:{session_id}] Enter code: "))?;
    match client.sign_in(&token, &code).await {
        Ok(_) => {}
        Err(SignInError::PasswordRequired(password)) => {
            let hint = password.hint().unwrap_or("none");
            let secret = rpassword::prompt_password(format!(
                "[session:{session_id}] Enter password (hint {hint}): "
            ))?;
            client
                .check_password(*password, secret.as_bytes())
                .await
                .map_err(|error| eyre!("{error}"))?;
        }
        Err(error) => return Err(eyre!("{error}")),
    }

    client
        .save_session()
        .await
        .map_err(|error| eyre!("{error}"))?;
    secure_session_file(session_file)?;
    println!("[session:{session_id}] Signed in.");
    Ok(())
}

async fn stop_api(api_task: &mut JoinHandle<Result<()>>) -> Vec<String> {
    match timeout(SHUTDOWN_TIMEOUT, &mut *api_task).await {
        Ok(Ok(Ok(()))) => Vec::new(),
        Ok(Ok(Err(error))) => vec![format!("API shutdown: {error:#}")],
        Ok(Err(error)) => vec![format!("API task shutdown: {error}")],
        Err(_) => {
            api_task.abort();
            let result = api_task.await;
            let mut errors = vec!["API graceful shutdown превысил 5 секунд".to_string()];
            if let Err(error) = result
                && !error.is_cancelled()
            {
                errors.push(format!("API task abort: {error}"));
            }
            errors
        }
    }
}

async fn save_and_cancel_connected(accounts: &[ConnectedAccount]) -> Vec<String> {
    if !accounts.is_empty() {
        println!("Saving {} Telegram session(s)...", accounts.len());
    }
    let mut errors = Vec::new();
    for account in accounts {
        if let Err(error) = account.client.save_session().await {
            errors.push(format!("session «{}»: save: {error}", account.config.id));
            continue;
        }
        if let Err(error) = secure_session_file(&account.config.file) {
            errors.push(format!(
                "session «{}»: protect file: {error:#}",
                account.config.id
            ));
        }
    }
    for account in accounts {
        account.shutdown.cancel();
    }
    errors
}

fn api_exit_error(result: std::result::Result<Result<()>, tokio::task::JoinError>) -> Report {
    match result {
        Ok(Ok(())) => eyre!("API server stopped unexpectedly"),
        Ok(Err(error)) => eyre!("API server stopped: {error:#}"),
        Err(error) => eyre!("API task failed: {error}"),
    }
}

fn worker_exit_error(result: Option<std::result::Result<SessionId, String>>) -> Report {
    match result {
        Some(Ok(session_id)) => {
            eyre!("update worker stopped unexpectedly: session «{session_id}»")
        }
        Some(Err(error)) => eyre!("update worker failed: {error}"),
        None => eyre!("update worker supervisor stopped unexpectedly"),
    }
}

fn with_cleanup(error: Report, cleanup_errors: Vec<String>) -> Report {
    if cleanup_errors.is_empty() {
        error
    } else {
        eyre!("{error:#}\nошибки cleanup:\n{}", cleanup_errors.join("\n"))
    }
}
