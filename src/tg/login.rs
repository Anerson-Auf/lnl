use color_eyre::eyre::{WrapErr, eyre};
use color_eyre::{Report, Result};
use ferogram::{Client, MtProxyConfig, SendCodeOutcome, ShutdownToken, SignInError};
use std::io::IsTerminal;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

use super::handlers::handle_update;
use super::history::seed_dialogues_from_folder;
use super::util::prompt;
use crate::api::{self, AppState, SessionState};
use crate::config::types::Telegram;
use crate::config::{CONFIG, Config, SessionConfig, SessionId, log_mtproxy, secure_session_file};

const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

struct RunningSession {
    state: Arc<SessionState>,
    file: PathBuf,
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

    let bind: SocketAddr = std::env::var("LNL_BIND")
        .unwrap_or_else(|_| "0.0.0.0:8080".into())
        .parse()
        .wrap_err("LNL_BIND")?;
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .wrap_err("не удалось открыть LNL_BIND")?;

    let update_stop = CancellationToken::new();
    let mut workers = JoinSet::new();
    let mut sessions = Vec::with_capacity(config.sessions.len());

    for session_config in &config.sessions {
        if let Some(error) = startup_worker_failure(&mut workers) {
            update_stop.cancel();
            let mut cleanup_errors = stop_workers(&mut workers).await;
            cleanup_errors.extend(save_and_cancel(&sessions).await);
            return Err(with_cleanup(error, cleanup_errors));
        }

        let running = match connect_session(config, session_config, mtproxy.as_ref()).await {
            Ok(running) => running,
            Err(error) => {
                update_stop.cancel();
                let mut cleanup_errors = stop_workers(&mut workers).await;
                cleanup_errors.extend(save_and_cancel(&sessions).await);
                return Err(with_cleanup(error, cleanup_errors));
            }
        };
        sessions.push(running);

        if let Some(error) = startup_worker_failure(&mut workers) {
            update_stop.cancel();
            let mut cleanup_errors = stop_workers(&mut workers).await;
            cleanup_errors.extend(save_and_cancel(&sessions).await);
            return Err(with_cleanup(error, cleanup_errors));
        }

        let state = Arc::clone(&sessions.last().expect("session was just inserted").state);
        let stop = update_stop.clone();
        workers.spawn(run_update_worker(state, stop));
    }

    let app_state = match AppState::new(
        sessions
            .iter()
            .map(|session| Arc::clone(&session.state))
            .collect(),
        config.default_session.clone(),
    ) {
        Ok(state) => state,
        Err(error) => {
            update_stop.cancel();
            let mut cleanup_errors = stop_workers(&mut workers).await;
            cleanup_errors.extend(save_and_cancel(&sessions).await);
            return Err(with_cleanup(eyre!(error), cleanup_errors));
        }
    };
    let api_stop = app_state.api_shutdown();
    let mut api_task = tokio::spawn(api::serve(app_state, listener, bind));

    println!(
        "Waiting for messages in {} session(s)... (Ctrl+C to quit)",
        sessions.len()
    );
    let mut api_finished = false;
    let primary_error = tokio::select! {
        signal = tokio::signal::ctrl_c() => {
            signal.err().map(|error| eyre!("Ctrl+C handler failed: {error}"))
        }
        result = &mut api_task => {
            api_finished = true;
            Some(api_exit_error(result))
        }
        result = workers.join_next() => {
            Some(worker_exit_error(result))
        }
    };

    api_stop.cancel();
    let mut cleanup_errors = Vec::new();
    if !api_finished {
        cleanup_errors.extend(stop_api(&mut api_task).await);
    }

    update_stop.cancel();
    cleanup_errors.extend(stop_workers(&mut workers).await);
    cleanup_errors.extend(save_and_cancel(&sessions).await);

    match primary_error {
        Some(error) => Err(with_cleanup(error, cleanup_errors)),
        None if cleanup_errors.is_empty() => Ok(()),
        None => Err(eyre!(
            "ошибки при остановке:\n{}",
            cleanup_errors.join("\n")
        )),
    }
}

async fn connect_session(
    config: &Config,
    session_config: &SessionConfig,
    mtproxy: Option<&MtProxyConfig>,
) -> Result<RunningSession> {
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
    let client = Arc::new(client);

    let setup = async {
        ensure_authorized(&client, &session_config.id, &session_config.file).await?;
        secure_session_file(&session_config.file)?;

        let telegram = Arc::new(Telegram {
            dialogues: dashmap::DashMap::new(),
        });
        println!(
            "[session:{}] Seeding folder «{}»…",
            session_config.id, session_config.folder
        );
        seed_dialogues_from_folder(&client, &telegram, &session_config.folder, 30)
            .await
            .wrap_err_with(|| format!("session «{}»: seed folder", session_config.id))?;
        println!(
            "[session:{}] Loaded {} dialogues from «{}»",
            session_config.id,
            telegram.dialogues.len(),
            session_config.folder
        );

        Ok(Arc::new(SessionState::new(
            session_config.id.clone(),
            Arc::clone(&client),
            telegram,
        )))
    }
    .await;

    match setup {
        Ok(state) => Ok(RunningSession {
            state,
            file: session_config.file.clone(),
            shutdown,
        }),
        Err(error) => {
            let mut cleanup_errors = Vec::new();
            if let Err(save_error) = client.save_session().await {
                cleanup_errors.push(format!(
                    "session «{}»: save after startup failure: {save_error}",
                    session_config.id
                ));
            } else if let Err(permission_error) = secure_session_file(&session_config.file) {
                cleanup_errors.push(format!(
                    "session «{}»: protect session file: {permission_error:#}",
                    session_config.id
                ));
            }
            shutdown.cancel();
            Err(with_cleanup(error, cleanup_errors))
        }
    }
}

async fn run_update_worker(state: Arc<SessionState>, stop: CancellationToken) -> Result<SessionId> {
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

async fn stop_workers(workers: &mut JoinSet<Result<SessionId>>) -> Vec<String> {
    match timeout(SHUTDOWN_TIMEOUT, drain_workers(workers, false)).await {
        Ok(errors) => errors,
        Err(_) => {
            workers.abort_all();
            let mut errors =
                vec!["Telegram update workers не остановились за 5 секунд".to_string()];
            errors.extend(drain_workers(workers, true).await);
            errors
        }
    }
}

async fn drain_workers(
    workers: &mut JoinSet<Result<SessionId>>,
    ignore_cancelled: bool,
) -> Vec<String> {
    let mut errors = Vec::new();
    while let Some(result) = workers.join_next().await {
        match result {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => errors.push(format!("update worker: {error:#}")),
            Err(error) if ignore_cancelled && error.is_cancelled() => {}
            Err(error) => errors.push(format!("update worker task: {error}")),
        }
    }
    errors
}

async fn save_and_cancel(sessions: &[RunningSession]) -> Vec<String> {
    if !sessions.is_empty() {
        println!("Saving {} Telegram session(s)...", sessions.len());
    }
    let mut errors = Vec::new();
    for session in sessions {
        if let Err(error) = session.state.client.save_session().await {
            errors.push(format!("session «{}»: save: {error}", session.state.id()));
            continue;
        }
        if let Err(error) = secure_session_file(&session.file) {
            errors.push(format!(
                "session «{}»: protect file: {error:#}",
                session.state.id()
            ));
        }
    }
    for session in sessions {
        session.shutdown.cancel();
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

fn worker_exit_error(
    result: Option<std::result::Result<Result<SessionId>, tokio::task::JoinError>>,
) -> Report {
    match result {
        Some(Ok(Ok(session_id))) => {
            eyre!("update worker stopped unexpectedly: session «{session_id}»")
        }
        Some(Ok(Err(error))) => error,
        Some(Err(error)) => eyre!("update worker task failed: {error}"),
        None => eyre!("all update workers stopped unexpectedly"),
    }
}

fn startup_worker_failure(workers: &mut JoinSet<Result<SessionId>>) -> Option<Report> {
    workers
        .try_join_next()
        .map(|result| worker_exit_error(Some(result)))
}

fn with_cleanup(error: Report, cleanup_errors: Vec<String>) -> Report {
    if cleanup_errors.is_empty() {
        error
    } else {
        eyre!("{error:#}\nошибки cleanup:\n{}", cleanup_errors.join("\n"))
    }
}
