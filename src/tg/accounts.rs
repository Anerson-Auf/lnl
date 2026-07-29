use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ferogram::media::ProfilePhoto;
use ferogram::tl;
use ferogram::{
    Client, ErrorKind, InvocationError, InvocationErrorExt, LoginToken, PasswordToken,
    SendCodeOutcome, ShutdownToken, SignInError,
};
use serde::Serialize;
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

use crate::api::{AppState, SessionState};
use crate::config::types::Telegram;
use crate::config::{SessionConfig, SessionId, secure_session_file};

use super::history::seed_dialogues_or_all;
use super::login::run_update_worker;

const AUTH_TTL: Duration = Duration::from_secs(10 * 60);
const TELEGRAM_TIMEOUT: Duration = Duration::from_secs(45);
const DIALOG_SYNC_TIMEOUT: Duration = Duration::from_secs(120);
const PHONE_WINDOW: Duration = Duration::from_secs(15 * 60);
const PHONE_ATTEMPTS_PER_SLOT: usize = 3;
const CHALLENGE_ATTEMPTS: u8 = 3;
const MAX_AVATAR_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Serialize)]
pub struct AccountSummary {
    pub id: SessionId,
    pub is_default: bool,
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phone_hint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dialog_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password_hint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<&'static str>,
}

#[derive(Clone, Serialize)]
pub struct AuthReply {
    pub id: SessionId,
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flow_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password_hint: Option<String>,
}

#[derive(Clone)]
pub struct Avatar {
    pub bytes: Arc<Vec<u8>>,
    pub content_type: &'static str,
    pub version: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccountErrorKind {
    NotFound,
    BadInput,
    Conflict,
    Expired,
    RateLimited,
    Telegram,
    Internal,
}

#[derive(Debug)]
pub struct AccountError {
    pub kind: AccountErrorKind,
    pub code: &'static str,
    pub message: &'static str,
    pub retry_after: Option<u64>,
}

impl AccountError {
    fn new(kind: AccountErrorKind, code: &'static str, message: &'static str) -> Self {
        Self {
            kind,
            code,
            message,
            retry_after: None,
        }
    }

    fn rate_limited(seconds: u64) -> Self {
        Self {
            kind: AccountErrorKind::RateLimited,
            code: "rate_limited",
            message: "Telegram просит подождать перед повтором",
            retry_after: Some(seconds.max(1)),
        }
    }
}

struct AccountIdentity {
    display_name: String,
    username: Option<String>,
    phone_hint: Option<String>,
}

enum Lifecycle {
    LoginRequired {
        error_code: Option<&'static str>,
    },
    RequestingCode {
        started: Instant,
    },
    CodeRequired {
        flow_id: String,
        token: LoginToken,
        expires: Instant,
        attempts: u8,
        busy: bool,
    },
    PasswordRequired {
        flow_id: String,
        token: Box<PasswordToken>,
        hint: Option<String>,
        expires: Instant,
        attempts: u8,
        busy: bool,
    },
    Syncing,
    Ready {
        identity: AccountIdentity,
        avatar: Option<Avatar>,
        dialog_count: usize,
    },
    Failed {
        code: &'static str,
    },
}

pub struct ManagedAccount {
    config: SessionConfig,
    client: Arc<Client>,
    shutdown: ShutdownToken,
    lifecycle: Mutex<Lifecycle>,
    phone_attempts: Mutex<VecDeque<Instant>>,
}

impl ManagedAccount {
    pub fn new(config: SessionConfig, client: Arc<Client>, shutdown: ShutdownToken) -> Self {
        Self {
            config,
            client,
            shutdown,
            lifecycle: Mutex::new(Lifecycle::LoginRequired { error_code: None }),
            phone_attempts: Mutex::new(VecDeque::new()),
        }
    }

    pub fn id(&self) -> &SessionId {
        &self.config.id
    }
}

pub struct AccountManager {
    accounts: HashMap<SessionId, Arc<ManagedAccount>>,
    order: Vec<SessionId>,
    default_session: SessionId,
    public: AppState<Client>,
    update_stop: CancellationToken,
    worker_results: mpsc::UnboundedSender<Result<SessionId, String>>,
    worker_handles: Mutex<Vec<JoinHandle<()>>>,
    finalizer_handles: Mutex<Vec<JoinHandle<()>>>,
}

impl AccountManager {
    pub fn new(
        accounts: Vec<Arc<ManagedAccount>>,
        default_session: SessionId,
        public: AppState<Client>,
        update_stop: CancellationToken,
        worker_results: mpsc::UnboundedSender<Result<SessionId, String>>,
    ) -> Result<Arc<Self>, String> {
        let mut registry = HashMap::with_capacity(accounts.len());
        let mut order = Vec::with_capacity(accounts.len());
        for account in accounts {
            let id = account.id().clone();
            if registry.insert(id.clone(), account).is_some() {
                return Err(format!("повтор account slot «{id}»"));
            }
            order.push(id);
        }
        if !registry.contains_key(&default_session) {
            return Err(format!(
                "default account slot «{default_session}» не настроен"
            ));
        }
        Ok(Arc::new(Self {
            accounts: registry,
            order,
            default_session,
            public,
            update_stop,
            worker_results,
            worker_handles: Mutex::new(Vec::new()),
            finalizer_handles: Mutex::new(Vec::new()),
        }))
    }

    pub async fn bootstrap(self: &Arc<Self>) -> Result<(), AccountError> {
        for id in &self.order {
            let account = self.account(id.as_str())?;
            let authorized = timeout(TELEGRAM_TIMEOUT, account.client.is_authorized())
                .await
                .map_err(|_| telegram_timeout())?
                .map_err(map_telegram_error)?;
            if authorized {
                self.finalize(account).await?;
            } else {
                *account.lifecycle.lock().await = Lifecycle::LoginRequired { error_code: None };
            }
        }
        Ok(())
    }

    pub async fn summaries(&self) -> Vec<AccountSummary> {
        let mut summaries = Vec::with_capacity(self.order.len());
        for id in &self.order {
            let Some(account) = self.accounts.get(id) else {
                continue;
            };
            summaries.push(summary_for(
                id,
                id == &self.default_session,
                &*account.lifecycle.lock().await,
            ));
        }
        summaries
    }

    pub async fn avatar(&self, id: &str) -> Result<Avatar, AccountError> {
        let account = self.account(id)?;
        let lifecycle = account.lifecycle.lock().await;
        match &*lifecycle {
            Lifecycle::Ready {
                avatar: Some(avatar),
                ..
            } => Ok(avatar.clone()),
            Lifecycle::Ready { avatar: None, .. } => Err(AccountError::new(
                AccountErrorKind::NotFound,
                "avatar_not_found",
                "У аккаунта нет аватара",
            )),
            _ => Err(AccountError::new(
                AccountErrorKind::Conflict,
                "account_not_ready",
                "Аккаунт ещё не готов",
            )),
        }
    }

    pub async fn request_phone(
        self: &Arc<Self>,
        id: &str,
        phone: &str,
    ) -> Result<AuthReply, AccountError> {
        let phone = normalize_phone(phone)?;
        let account = self.account(id)?;
        check_phone_rate(&account).await?;

        {
            let mut lifecycle = account.lifecycle.lock().await;
            match &*lifecycle {
                Lifecycle::LoginRequired { .. } | Lifecycle::Failed { .. } => {}
                Lifecycle::RequestingCode { started } if started.elapsed() >= AUTH_TTL => {}
                Lifecycle::CodeRequired { expires, .. }
                | Lifecycle::PasswordRequired { expires, .. }
                    if Instant::now() >= *expires => {}
                _ => {
                    return Err(AccountError::new(
                        AccountErrorKind::Conflict,
                        "auth_in_progress",
                        "Авторизация этого аккаунта уже начата",
                    ));
                }
            }
            *lifecycle = Lifecycle::RequestingCode {
                started: Instant::now(),
            };
        }

        let already_authorized =
            match timeout(TELEGRAM_TIMEOUT, account.client.is_authorized()).await {
                Ok(Ok(authorized)) => authorized,
                Ok(Err(error)) => {
                    let error = map_telegram_error(error);
                    *account.lifecycle.lock().await = Lifecycle::LoginRequired {
                        error_code: Some(error.code),
                    };
                    return Err(error);
                }
                Err(_) => {
                    let error = telegram_timeout();
                    *account.lifecycle.lock().await = Lifecycle::LoginRequired {
                        error_code: Some(error.code),
                    };
                    return Err(error);
                }
            };
        if already_authorized {
            self.start_finalize(account).await?;
            return Ok(AuthReply {
                id: id.parse().expect("configured id is valid"),
                status: "syncing",
                flow_id: None,
                password_hint: None,
            });
        }

        let outcome = timeout(TELEGRAM_TIMEOUT, account.client.request_login_code(&phone))
            .await
            .map_err(|_| telegram_timeout())?
            .map_err(map_telegram_error);

        match outcome {
            Ok(SendCodeOutcome::CodeRequired(token)) => {
                let flow_id = random_flow_id()?;
                *account.lifecycle.lock().await = Lifecycle::CodeRequired {
                    flow_id: flow_id.clone(),
                    token,
                    expires: Instant::now() + AUTH_TTL,
                    attempts: 0,
                    busy: false,
                };
                Ok(AuthReply {
                    id: id.parse().expect("configured id is valid"),
                    status: "code_required",
                    flow_id: Some(flow_id),
                    password_hint: None,
                })
            }
            Ok(SendCodeOutcome::AlreadyAuthorized(_)) => {
                self.start_finalize(account).await?;
                Ok(AuthReply {
                    id: id.parse().expect("configured id is valid"),
                    status: "syncing",
                    flow_id: None,
                    password_hint: None,
                })
            }
            Err(error) => {
                *account.lifecycle.lock().await = Lifecycle::LoginRequired {
                    error_code: Some(error.code),
                };
                Err(error)
            }
        }
    }

    pub async fn submit_code(
        self: &Arc<Self>,
        id: &str,
        flow_id: &str,
        code: &str,
    ) -> Result<AuthReply, AccountError> {
        let code = validate_code(code)?;
        validate_flow_id(flow_id)?;
        let account = self.account(id)?;

        let (token, attempts, expires) = {
            let mut lifecycle = account.lifecycle.lock().await;
            let Lifecycle::CodeRequired {
                flow_id: expected,
                token,
                expires,
                attempts,
                busy,
            } = &mut *lifecycle
            else {
                return Err(challenge_conflict());
            };
            if expected != flow_id {
                return Err(challenge_conflict());
            }
            if Instant::now() >= *expires {
                *lifecycle = Lifecycle::LoginRequired {
                    error_code: Some("auth_expired"),
                };
                return Err(challenge_expired());
            }
            if *busy {
                return Err(challenge_conflict());
            }
            *busy = true;
            (token.clone(), *attempts, *expires)
        };

        let result = timeout(TELEGRAM_TIMEOUT, account.client.sign_in(&token, &code))
            .await
            .map_err(|_| telegram_timeout());

        match result {
            Ok(Ok(_)) => {
                self.start_finalize(account).await?;
                Ok(AuthReply {
                    id: id.parse().expect("configured id is valid"),
                    status: "syncing",
                    flow_id: None,
                    password_hint: None,
                })
            }
            Ok(Err(SignInError::PasswordRequired(password))) => {
                let hint = password.hint().map(str::to_string);
                *account.lifecycle.lock().await = Lifecycle::PasswordRequired {
                    flow_id: flow_id.to_string(),
                    token: password,
                    hint: hint.clone(),
                    expires,
                    attempts: 0,
                    busy: false,
                };
                Ok(AuthReply {
                    id: id.parse().expect("configured id is valid"),
                    status: "password_required",
                    flow_id: Some(flow_id.to_string()),
                    password_hint: hint,
                })
            }
            Ok(Err(SignInError::InvalidCode)) => {
                let next_attempt = attempts.saturating_add(1);
                if next_attempt >= CHALLENGE_ATTEMPTS {
                    *account.lifecycle.lock().await = Lifecycle::LoginRequired {
                        error_code: Some("code_attempts_exhausted"),
                    };
                    return Err(AccountError::new(
                        AccountErrorKind::Expired,
                        "code_attempts_exhausted",
                        "Слишком много неверных кодов — запроси новый",
                    ));
                }
                *account.lifecycle.lock().await = Lifecycle::CodeRequired {
                    flow_id: flow_id.to_string(),
                    token,
                    expires,
                    attempts: next_attempt,
                    busy: false,
                };
                Err(AccountError::new(
                    AccountErrorKind::BadInput,
                    "invalid_code",
                    "Код неверный или устарел",
                ))
            }
            Ok(Err(SignInError::SignUpRequired)) => {
                *account.lifecycle.lock().await = Lifecycle::LoginRequired {
                    error_code: Some("signup_required"),
                };
                Err(AccountError::new(
                    AccountErrorKind::BadInput,
                    "signup_required",
                    "Сначала зарегистрируй номер в официальном Telegram",
                ))
            }
            Ok(Err(SignInError::Other(error))) => {
                let error = map_telegram_error(error);
                restore_code(&account, flow_id, token, expires, attempts).await;
                Err(error)
            }
            Err(error) => {
                restore_code(&account, flow_id, token, expires, attempts).await;
                Err(error)
            }
        }
    }

    pub async fn submit_password(
        self: &Arc<Self>,
        id: &str,
        flow_id: &str,
        password: &str,
    ) -> Result<AuthReply, AccountError> {
        validate_password(password)?;
        validate_flow_id(flow_id)?;
        let account = self.account(id)?;

        let (token, hint, attempts, expires) = {
            let mut lifecycle = account.lifecycle.lock().await;
            let Lifecycle::PasswordRequired {
                flow_id: expected,
                token,
                hint,
                expires,
                attempts,
                busy,
            } = &mut *lifecycle
            else {
                return Err(challenge_conflict());
            };
            if expected != flow_id {
                return Err(challenge_conflict());
            }
            if Instant::now() >= *expires {
                *lifecycle = Lifecycle::LoginRequired {
                    error_code: Some("auth_expired"),
                };
                return Err(challenge_expired());
            }
            if *busy {
                return Err(challenge_conflict());
            }
            *busy = true;
            (token.clone(), hint.clone(), *attempts, *expires)
        };

        let result = timeout(
            TELEGRAM_TIMEOUT,
            account
                .client
                .check_password(token.as_ref().clone(), password.as_bytes()),
        )
        .await
        .map_err(|_| telegram_timeout());

        match result {
            Ok(Ok(_)) => {
                self.start_finalize(account).await?;
                Ok(AuthReply {
                    id: id.parse().expect("configured id is valid"),
                    status: "syncing",
                    flow_id: None,
                    password_hint: None,
                })
            }
            Ok(Err(error)) if is_invalid_password(&error) => {
                let next_attempt = attempts.saturating_add(1);
                if next_attempt >= CHALLENGE_ATTEMPTS {
                    *account.lifecycle.lock().await = Lifecycle::LoginRequired {
                        error_code: Some("password_attempts_exhausted"),
                    };
                    return Err(AccountError::new(
                        AccountErrorKind::Expired,
                        "password_attempts_exhausted",
                        "Слишком много попыток — начни вход заново",
                    ));
                }
                *account.lifecycle.lock().await = Lifecycle::PasswordRequired {
                    flow_id: flow_id.to_string(),
                    token,
                    hint,
                    expires,
                    attempts: next_attempt,
                    busy: false,
                };
                Err(AccountError::new(
                    AccountErrorKind::BadInput,
                    "invalid_password",
                    "Неверный пароль двухэтапной аутентификации",
                ))
            }
            Ok(Err(error)) => {
                let error = map_telegram_error(error);
                restore_password(&account, flow_id, token, hint, expires, attempts).await;
                Err(error)
            }
            Err(error) => {
                restore_password(&account, flow_id, token, hint, expires, attempts).await;
                Err(error)
            }
        }
    }

    pub async fn shutdown(&self) -> Vec<String> {
        self.update_stop.cancel();
        let mut errors = Vec::new();
        let mut finalizers = self.finalizer_handles.lock().await;
        for mut handle in finalizers.drain(..) {
            match timeout(Duration::from_secs(5), &mut handle).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => errors.push(format!("account finalizer join: {error}")),
                Err(_) => {
                    handle.abort();
                    errors.push("account finalizer stop timeout".to_string());
                }
            }
        }
        drop(finalizers);

        let mut handles = self.worker_handles.lock().await;
        for mut handle in handles.drain(..) {
            match timeout(Duration::from_secs(5), &mut handle).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => errors.push(format!("update worker join: {error}")),
                Err(_) => {
                    handle.abort();
                    errors.push("update worker stop timeout".to_string());
                }
            }
        }
        drop(handles);

        for id in &self.order {
            let Some(account) = self.accounts.get(id) else {
                continue;
            };
            if let Err(error) = account.client.save_session().await {
                errors.push(format!("session «{id}»: save: {error}"));
            } else if let Err(error) = secure_session_file(&account.config.file) {
                errors.push(format!("session «{id}»: protect: {error:#}"));
            }
            account.shutdown.cancel();
        }
        errors
    }

    fn account(&self, id: &str) -> Result<Arc<ManagedAccount>, AccountError> {
        self.accounts.get(id).cloned().ok_or_else(|| {
            AccountError::new(
                AccountErrorKind::NotFound,
                "account_not_found",
                "Аккаунт не настроен",
            )
        })
    }

    async fn finalize(self: &Arc<Self>, account: Arc<ManagedAccount>) -> Result<(), AccountError> {
        {
            let mut lifecycle = account.lifecycle.lock().await;
            if matches!(&*lifecycle, Lifecycle::Ready { .. }) {
                return Ok(());
            }
            if matches!(&*lifecycle, Lifecycle::Syncing) {
                return Err(challenge_conflict());
            }
            *lifecycle = Lifecycle::Syncing;
        }

        let result = self.hydrate_account(&account).await;
        if let Err(error) = &result {
            *account.lifecycle.lock().await = Lifecycle::Failed { code: error.code };
        }
        result
    }

    async fn start_finalize(
        self: &Arc<Self>,
        account: Arc<ManagedAccount>,
    ) -> Result<(), AccountError> {
        {
            let mut lifecycle = account.lifecycle.lock().await;
            if matches!(&*lifecycle, Lifecycle::Ready { .. } | Lifecycle::Syncing) {
                return Ok(());
            }
            *lifecycle = Lifecycle::Syncing;
        }

        let manager = Arc::clone(self);
        let handle = tokio::spawn(async move {
            if let Err(error) = manager.hydrate_account(&account).await {
                *account.lifecycle.lock().await = Lifecycle::Failed { code: error.code };
            }
        });
        self.finalizer_handles.lock().await.push(handle);
        Ok(())
    }

    async fn hydrate_account(&self, account: &Arc<ManagedAccount>) -> Result<(), AccountError> {
        timeout(TELEGRAM_TIMEOUT, account.client.save_session())
            .await
            .map_err(|_| telegram_timeout())?
            .map_err(map_telegram_error)?;
        secure_session_file(&account.config.file).map_err(|_| {
            AccountError::new(
                AccountErrorKind::Internal,
                "session_permissions",
                "Не удалось защитить файл Telegram-сессии",
            )
        })?;

        let me = timeout(TELEGRAM_TIMEOUT, account.client.get_me())
            .await
            .map_err(|_| telegram_timeout())?
            .map_err(map_telegram_error)?;
        let identity = identity_from_user(&me, account.id());
        let avatar = load_avatar(&account.client, &me).await;

        let telegram = Arc::new(Telegram {
            dialogues: dashmap::DashMap::new(),
        });
        let source = timeout(
            DIALOG_SYNC_TIMEOUT,
            seed_dialogues_or_all(&account.client, &telegram, &account.config.folder, 30),
        )
        .await
        .map_err(|_| {
            AccountError::new(
                AccountErrorKind::Telegram,
                "dialog_sync_timeout",
                "Telegram слишком долго загружает диалоги",
            )
        })?
        .map_err(|_| {
            AccountError::new(
                AccountErrorKind::Telegram,
                "dialog_sync_failed",
                "Не удалось загрузить диалоги Telegram",
            )
        })?;

        let dialog_count = telegram.dialogues.len();
        println!(
            "[session:{}] Loaded {dialog_count} dialogues ({source:?})",
            account.id()
        );
        let session = Arc::new(SessionState::new(
            account.id().clone(),
            Arc::clone(&account.client),
            telegram,
        ));
        self.public
            .insert_session(Arc::clone(&session))
            .map_err(|_| {
                AccountError::new(
                    AccountErrorKind::Conflict,
                    "account_already_ready",
                    "Аккаунт уже подключён",
                )
            })?;
        self.spawn_worker(session).await;
        *account.lifecycle.lock().await = Lifecycle::Ready {
            identity,
            avatar,
            dialog_count,
        };
        Ok(())
    }

    async fn spawn_worker(&self, session: Arc<SessionState>) {
        let stop = self.update_stop.clone();
        let results = self.worker_results.clone();
        let handle = tokio::spawn(async move {
            let result = run_update_worker(session, stop)
                .await
                .map_err(|error| format!("{error:#}"));
            let _ = results.send(result);
        });
        self.worker_handles.lock().await.push(handle);
    }
}

fn summary_for(id: &SessionId, is_default: bool, lifecycle: &Lifecycle) -> AccountSummary {
    let mut summary = AccountSummary {
        id: id.clone(),
        is_default,
        status: "login_required",
        display_name: None,
        username: None,
        phone_hint: None,
        avatar_url: None,
        dialog_count: None,
        password_hint: None,
        error_code: None,
    };
    match lifecycle {
        Lifecycle::LoginRequired { error_code } => summary.error_code = *error_code,
        Lifecycle::RequestingCode { .. } | Lifecycle::Syncing => summary.status = "syncing",
        Lifecycle::CodeRequired { .. } => summary.status = "code_required",
        Lifecycle::PasswordRequired { hint, .. } => {
            summary.status = "password_required";
            summary.password_hint.clone_from(hint);
        }
        Lifecycle::Ready {
            identity,
            avatar,
            dialog_count,
        } => {
            summary.status = "ready";
            summary.display_name = Some(identity.display_name.clone());
            summary.username.clone_from(&identity.username);
            summary.phone_hint.clone_from(&identity.phone_hint);
            summary.avatar_url = avatar
                .as_ref()
                .map(|_| format!("/api/admin/accounts/{id}/avatar"));
            summary.dialog_count = Some(*dialog_count);
        }
        Lifecycle::Failed { code } => {
            summary.status = "error";
            summary.error_code = Some(*code);
        }
    }
    summary
}

fn identity_from_user(user: &tl::types::User, id: &SessionId) -> AccountIdentity {
    let first = user.first_name.as_deref().unwrap_or("");
    let last = user.last_name.as_deref().unwrap_or("");
    let mut display_name = format!("{first} {last}").trim().to_string();
    if display_name.is_empty() {
        display_name = user
            .username
            .as_ref()
            .map(|username| format!("@{username}"))
            .unwrap_or_else(|| id.to_string());
    }
    AccountIdentity {
        display_name,
        username: user.username.clone(),
        phone_hint: user.phone.as_deref().and_then(mask_phone),
    }
}

async fn load_avatar(client: &Client, user: &tl::types::User) -> Option<Avatar> {
    let photo = user.photo.as_ref()?;
    let version = match photo {
        tl::enums::UserProfilePhoto::UserProfilePhoto(photo) => photo.photo_id,
        tl::enums::UserProfilePhoto::Empty => return None,
    };
    let photo = ProfilePhoto::from_user(tl::enums::InputPeer::PeerSelf, photo)?.small();
    let mut bytes = Vec::new();
    timeout(TELEGRAM_TIMEOUT, client.download(&photo, &mut bytes, None))
        .await
        .ok()?
        .ok()?;
    if bytes.is_empty() || bytes.len() > MAX_AVATAR_BYTES {
        return None;
    }
    let content_type = image_content_type(&bytes)?;
    Some(Avatar {
        bytes: Arc::new(bytes),
        content_type,
        version,
    })
}

fn image_content_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some("image/jpeg");
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("image/png");
    }
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    None
}

async fn check_phone_rate(account: &ManagedAccount) -> Result<(), AccountError> {
    let now = Instant::now();
    let mut attempts = account.phone_attempts.lock().await;
    while attempts
        .front()
        .is_some_and(|attempt| now.duration_since(*attempt) >= PHONE_WINDOW)
    {
        attempts.pop_front();
    }
    if attempts.len() >= PHONE_ATTEMPTS_PER_SLOT {
        let retry_after = PHONE_WINDOW
            .saturating_sub(now.duration_since(*attempts.front().expect("not empty")))
            .as_secs();
        return Err(AccountError::rate_limited(retry_after));
    }
    attempts.push_back(now);
    Ok(())
}

async fn restore_code(
    account: &ManagedAccount,
    flow_id: &str,
    token: LoginToken,
    expires: Instant,
    attempts: u8,
) {
    *account.lifecycle.lock().await = Lifecycle::CodeRequired {
        flow_id: flow_id.to_string(),
        token,
        expires,
        attempts,
        busy: false,
    };
}

async fn restore_password(
    account: &ManagedAccount,
    flow_id: &str,
    token: Box<PasswordToken>,
    hint: Option<String>,
    expires: Instant,
    attempts: u8,
) {
    *account.lifecycle.lock().await = Lifecycle::PasswordRequired {
        flow_id: flow_id.to_string(),
        token,
        hint,
        expires,
        attempts,
        busy: false,
    };
}

fn normalize_phone(raw: &str) -> Result<String, AccountError> {
    if raw.chars().any(char::is_control) {
        return Err(invalid_phone());
    }
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(invalid_phone());
    }
    let mut digits = String::with_capacity(raw.len());
    for (index, character) in raw.chars().enumerate() {
        match character {
            '+' if index == 0 => {}
            '0'..='9' => digits.push(character),
            ' ' | '-' | '(' | ')' => {}
            _ => return Err(invalid_phone()),
        }
    }
    if !(8..=15).contains(&digits.len()) {
        return Err(invalid_phone());
    }
    Ok(format!("+{digits}"))
}

fn validate_code(raw: &str) -> Result<String, AccountError> {
    if raw.chars().any(char::is_control) {
        return Err(AccountError::new(
            AccountErrorKind::BadInput,
            "invalid_code_format",
            "Проверь формат кода Telegram",
        ));
    }
    let code = raw.trim();
    if code.is_empty() || code.len() > 64 {
        return Err(AccountError::new(
            AccountErrorKind::BadInput,
            "invalid_code_format",
            "Проверь формат кода Telegram",
        ));
    }
    Ok(code.to_string())
}

fn validate_password(password: &str) -> Result<(), AccountError> {
    if password.is_empty() || password.len() > 1024 || password.chars().any(char::is_control) {
        return Err(AccountError::new(
            AccountErrorKind::BadInput,
            "invalid_password_format",
            "Проверь формат пароля",
        ));
    }
    Ok(())
}

fn validate_flow_id(flow_id: &str) -> Result<(), AccountError> {
    if flow_id.len() != 32 || !flow_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(challenge_conflict());
    }
    Ok(())
}

fn random_flow_id() -> Result<String, AccountError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| {
        AccountError::new(
            AccountErrorKind::Internal,
            "random_unavailable",
            "Не удалось начать безопасную авторизацию",
        )
    })?;
    let mut output = String::with_capacity(32);
    for byte in bytes {
        use std::fmt::Write;
        write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
    }
    Ok(output)
}

fn mask_phone(phone: &str) -> Option<String> {
    let digits = phone
        .chars()
        .filter(char::is_ascii_digit)
        .collect::<String>();
    if digits.len() < 4 {
        return None;
    }
    Some(format!("•••• {}", &digits[digits.len() - 4..]))
}

fn map_telegram_error(error: InvocationError) -> AccountError {
    match error.kind() {
        ErrorKind::FloodWait(seconds) => AccountError::rate_limited(seconds),
        ErrorKind::Network => AccountError::new(
            AccountErrorKind::Telegram,
            "telegram_unavailable",
            "Telegram сейчас недоступен — попробуй ещё раз",
        ),
        ErrorKind::Auth => AccountError::new(
            AccountErrorKind::Telegram,
            "telegram_auth_failed",
            "Telegram отклонил авторизацию",
        ),
        _ => AccountError::new(
            AccountErrorKind::Telegram,
            "telegram_error",
            "Telegram не смог выполнить запрос",
        ),
    }
}

fn is_invalid_password(error: &InvocationError) -> bool {
    matches!(
        error,
        InvocationError::Rpc(rpc) if rpc.name == "PASSWORD_HASH_INVALID"
    )
}

fn invalid_phone() -> AccountError {
    AccountError::new(
        AccountErrorKind::BadInput,
        "invalid_phone",
        "Введи номер в международном формате",
    )
}

fn challenge_conflict() -> AccountError {
    AccountError::new(
        AccountErrorKind::Conflict,
        "auth_state_conflict",
        "Этот шаг авторизации уже недействителен",
    )
}

fn challenge_expired() -> AccountError {
    AccountError::new(
        AccountErrorKind::Expired,
        "auth_expired",
        "Время авторизации истекло — начни заново",
    )
}

fn telegram_timeout() -> AccountError {
    AccountError::new(
        AccountErrorKind::Telegram,
        "telegram_timeout",
        "Telegram слишком долго не отвечает",
    )
}

#[cfg(test)]
mod tests {
    use super::{image_content_type, mask_phone, normalize_phone, validate_code, validate_flow_id};

    #[test]
    fn phone_is_normalized_without_accepting_arbitrary_text() {
        assert_eq!(
            normalize_phone("+7 (999) 123-45-67").unwrap(),
            "+79991234567"
        );
        assert!(normalize_phone("+7 hello").is_err());
        assert!(normalize_phone("123").is_err());
    }

    #[test]
    fn auth_inputs_are_bounded() {
        assert!(validate_code("word phrase").is_ok());
        assert!(validate_code("\n123").is_err());
        assert!(validate_code(&"1".repeat(65)).is_err());
        assert!(validate_flow_id("00112233445566778899aabbccddeeff").is_ok());
        assert!(validate_flow_id("../session").is_err());
    }

    #[test]
    fn profile_fields_are_minimized_and_avatar_magic_is_checked() {
        assert_eq!(mask_phone("+79991234567").as_deref(), Some("•••• 4567"));
        assert_eq!(
            image_content_type(&[0xff, 0xd8, 0xff, 0]),
            Some("image/jpeg")
        );
        assert_eq!(image_content_type(b"<svg></svg>"), None);
    }
}
