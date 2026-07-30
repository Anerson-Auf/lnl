pub mod types;
use color_eyre::{Result, eyre::WrapErr, eyre::eyre};
use ferogram::{MtProxyConfig, TransportKind, parse_proxy_link};
use serde::Serialize;
use std::borrow::Borrow;
use std::collections::HashSet;
use std::fmt;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};
use std::str::FromStr;

const LEGACY_SESSION_ID: &str = "default";
const LEGACY_SESSION_FILE: &str = "lnl.session";
const MAX_SESSION_ID_LEN: usize = 32;

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct SessionId(String);

impl SessionId {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn env_key(&self, suffix: &str) -> String {
        format!("TG_SESSION_{}_{}", self.0.to_ascii_uppercase(), suffix)
    }
}

impl Borrow<str> for SessionId {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for SessionId {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        if value.is_empty() {
            return Err("id сессии пуст".to_string());
        }
        if value.len() > MAX_SESSION_ID_LEN {
            return Err(format!("id сессии длиннее {MAX_SESSION_ID_LEN} символов"));
        }
        if !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err("id сессии должен содержать только строчные a-z, цифры и _".to_string());
        }
        if !value.as_bytes()[0].is_ascii_lowercase() && !value.as_bytes()[0].is_ascii_digit() {
            return Err("id сессии должен начинаться с a-z или цифры".to_string());
        }
        Ok(Self(value.to_string()))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionConfig {
    pub id: SessionId,
    pub file: PathBuf,
    pub folder: String,
}

pub struct Config {
    pub api_id: i32,
    pub api_hash: String,
    /// Полная `https://t.me/proxy?...` или пусто = без прокси (VPS).
    pub tg_proxy_link: String,
    /// `host:port` или `socks5://host:port` — выход через VPS без MTProxy-протокола.
    pub tg_socks5: String,
    pub tg_proxy_sni: String,
    pub sessions: Vec<SessionConfig>,
    pub default_session: SessionId,
}

pub static CONFIG: std::sync::OnceLock<Config> = std::sync::OnceLock::new();

#[cfg(unix)]
pub fn harden_process_file_creation() {
    // Ferogram replaces session files atomically, so every future rewrite must inherit 0600.
    unsafe {
        libc::umask(0o077);
    }
}

#[cfg(not(unix))]
pub fn harden_process_file_creation() {}

impl Config {
    pub fn from_env() -> Result<Self> {
        let cwd = std::env::current_dir().wrap_err("не удалось определить текущую папку")?;
        Self::from_values(|key| std::env::var(key).ok(), &cwd)
    }

    fn from_values(mut value: impl FnMut(&str) -> Option<String>, cwd: &Path) -> Result<Self> {
        let api_id: i32 = required_value(&mut value, "api_id")?
            .parse()
            .wrap_err("api_id должен быть числом")?;
        if api_id <= 0 {
            return Err(eyre!("api_id должен быть положительным числом"));
        }
        let api_hash = required_value(&mut value, "api_hash")?;
        let tg_proxy_link = value("TG_PROXY_LINK").unwrap_or_default();
        let tg_socks5 = value("TG_SOCKS5").unwrap_or_default();
        let tg_proxy_sni = value("TG_PROXY_SNI").unwrap_or_default();

        let ids = parse_session_ids(value("TG_SESSIONS").as_deref())?;
        let default_session = match value("TG_DEFAULT_SESSION") {
            Some(raw) => raw
                .trim()
                .parse()
                .map_err(|error: String| eyre!("TG_DEFAULT_SESSION: {error}"))?,
            None => ids
                .iter()
                .find(|id| id.as_str() == LEGACY_SESSION_ID)
                .cloned()
                .unwrap_or_else(|| ids[0].clone()),
        };
        if !ids.contains(&default_session) {
            return Err(eyre!(
                "TG_DEFAULT_SESSION={default_session}: такой id отсутствует в TG_SESSIONS"
            ));
        }

        let session_dir = match value("TG_SESSION_DIR") {
            Some(raw) if raw.trim().is_empty() => {
                return Err(eyre!("TG_SESSION_DIR задан, но пуст"));
            }
            Some(raw) => absolute_path(cwd, Path::new(raw.trim())),
            None => absolute_path(cwd, Path::new(".")),
        };
        let global_folder = value("TG_FOLDER");
        let legacy_file = value("TG_SESSION_FILE");
        if legacy_file
            .as_deref()
            .is_some_and(|raw| raw.trim().is_empty())
        {
            return Err(eyre!("TG_SESSION_FILE задан, но пуст"));
        }
        if legacy_file.is_some() && !ids.iter().any(|id| id.as_str() == LEGACY_SESSION_ID) {
            return Err(eyre!(
                "TG_SESSION_FILE задан, но сессии «{LEGACY_SESSION_ID}» нет в TG_SESSIONS"
            ));
        }

        let mut sessions = Vec::with_capacity(ids.len());
        let mut targets = HashSet::new();
        let mut stems = HashSet::new();
        for id in ids {
            let folder_key = id.env_key("FOLDER");
            let folder = value(&folder_key)
                .or_else(|| global_folder.clone())
                .unwrap_or_else(|| "all".to_string());
            let folder = folder.trim();
            if folder.chars().any(char::is_control) {
                return Err(eyre!("папка сессии «{id}» содержит управляющие символы"));
            }
            let folder = if folder.is_empty() { "all" } else { folder };

            let file_key = id.env_key("FILE");
            let explicit_file = value(&file_key).or_else(|| {
                (id.as_str() == LEGACY_SESSION_ID)
                    .then(|| legacy_file.clone())
                    .flatten()
            });
            let file = match explicit_file {
                Some(raw) if raw.trim().is_empty() => {
                    return Err(eyre!("{file_key} задан, но пуст"));
                }
                Some(raw) => absolute_path(&session_dir, Path::new(raw.trim())),
                None => {
                    let name = if id.as_str() == LEGACY_SESSION_ID {
                        LEGACY_SESSION_FILE.to_string()
                    } else {
                        format!("lnl.{id}.session")
                    };
                    absolute_path(&session_dir, Path::new(&name))
                }
            };
            if file
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| {
                    extension.eq_ignore_ascii_case("bak") || extension.eq_ignore_ascii_case("tmp")
                })
            {
                return Err(eyre!(
                    "{file_key}: расширения .bak и .tmp зарезервированы Ferogram"
                ));
            }

            if !targets.insert(path_collision_key(&file)) {
                return Err(eyre!(
                    "несколько Telegram-сессий используют один файл {}",
                    file.display()
                ));
            }
            let stem = file.with_extension("");
            if !stems.insert(path_collision_key(&stem)) {
                return Err(eyre!(
                    "файлы Telegram-сессий имеют конфликтующие служебные имена: {}",
                    file.display()
                ));
            }

            sessions.push(SessionConfig {
                id,
                file,
                folder: folder.to_string(),
            });
        }

        Ok(Self {
            api_id,
            api_hash,
            tg_proxy_link,
            tg_socks5,
            tg_proxy_sni,
            sessions,
            default_session,
        })
    }

    pub fn prepare_session_files(&self) -> Result<()> {
        let mut targets = HashSet::new();
        let mut stems = HashSet::new();
        #[cfg(unix)]
        let mut identities = HashSet::new();

        for session in &self.sessions {
            let parent = session
                .file
                .parent()
                .ok_or_else(|| eyre!("нет родительской папки для {}", session.file.display()))?;
            std::fs::create_dir_all(parent).wrap_err_with(|| {
                format!(
                    "не удалось создать папку сессии «{}»: {}",
                    session.id,
                    parent.display()
                )
            })?;
            let parent = parent
                .canonicalize()
                .wrap_err_with(|| format!("не удалось проверить папку сессии «{}»", session.id))?;
            let file_name = session.file.file_name().ok_or_else(|| {
                eyre!(
                    "некорректный файл сессии «{}»: {}",
                    session.id,
                    session.file.display()
                )
            })?;
            let target = parent.join(file_name);
            let checked_target = match std::fs::symlink_metadata(&target) {
                Ok(metadata) => {
                    if metadata.file_type().is_symlink() {
                        return Err(eyre!(
                            "файл сессии «{}» не может быть symlink: {}",
                            session.id,
                            target.display()
                        ));
                    }
                    if !metadata.is_file() {
                        return Err(eyre!(
                            "путь сессии «{}» не является файлом: {}",
                            session.id,
                            target.display()
                        ));
                    }
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::MetadataExt;

                        if !identities.insert((metadata.dev(), metadata.ino())) {
                            return Err(eyre!(
                                "файл сессии «{}» является hardlink другой сессии: {}",
                                session.id,
                                target.display()
                            ));
                        }
                    }
                    secure_session_file(&target)?;
                    target.canonicalize().wrap_err_with(|| {
                        format!("не удалось проверить файл сессии «{}»", session.id)
                    })?
                }
                Err(error) if error.kind() == ErrorKind::NotFound => target.clone(),
                Err(error) => {
                    return Err(error).wrap_err_with(|| {
                        format!(
                            "не удалось проверить файл сессии «{}»: {}",
                            session.id,
                            target.display()
                        )
                    });
                }
            };
            if !targets.insert(path_collision_key(&checked_target))
                || !stems.insert(path_collision_key(&checked_target.with_extension("")))
            {
                return Err(eyre!(
                    "файл сессии «{}» пересекается с другой сессией: {}",
                    session.id,
                    target.display()
                ));
            }
        }
        Ok(())
    }

    pub fn mtproxy(&self) -> Result<Option<MtProxyConfig>> {
        let url = self.tg_proxy_link.trim();
        if url.is_empty() {
            return Ok(None);
        }

        let mut mp = parse_proxy_link(url).ok_or_else(|| {
            eyre!("TG_PROXY_LINK: не разобрать ссылку (нужен t.me/proxy?server=&port=&secret=)")
        })?;

        if let TransportKind::FakeTls { ref mut domain, .. } = mp.transport
            && !self.tg_proxy_sni.trim().is_empty()
        {
            *domain = self.tg_proxy_sni.trim().to_string();
        }

        Ok(Some(mp))
    }
}

fn required_value(value: &mut impl FnMut(&str) -> Option<String>, key: &str) -> Result<String> {
    let raw = value(key).ok_or_else(|| eyre!("нет {key} в env / .env"))?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(eyre!("{key} задан, но пуст"));
    }
    Ok(trimmed.to_string())
}

fn parse_session_ids(raw: Option<&str>) -> Result<Vec<SessionId>> {
    let Some(raw) = raw else {
        return Ok(vec![
            LEGACY_SESSION_ID
                .parse()
                .expect("legacy session id is valid"),
        ]);
    };
    if raw.trim().is_empty() {
        return Err(eyre!("TG_SESSIONS задан, но пуст"));
    }

    let mut seen = HashSet::new();
    let mut ids = Vec::new();
    for token in raw.split(',') {
        let token = token.trim();
        if token.is_empty() {
            return Err(eyre!("TG_SESSIONS содержит пустой id"));
        }
        let id: SessionId = token
            .parse()
            .map_err(|error: String| eyre!("TG_SESSIONS «{token}»: {error}"))?;
        if !seen.insert(id.clone()) {
            return Err(eyre!("TG_SESSIONS содержит повтор «{id}»"));
        }
        ids.push(id);
    }
    Ok(ids)
}

fn absolute_path(base: &Path, path: &Path) -> PathBuf {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    normalize_path(&joined)
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn path_collision_key(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

#[cfg(unix)]
pub(crate) fn secure_session_file(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(eyre!(
                "session-файл не может быть symlink: {}",
                path.display()
            ));
        }
        Ok(metadata) if metadata.is_file() => metadata,
        Ok(_) => {
            return Err(eyre!("session-путь не является файлом: {}", path.display()));
        }
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error)
                .wrap_err_with(|| format!("не удалось прочитать permissions {}", path.display()));
        }
    };
    let mut permissions = metadata.permissions();
    permissions.set_mode(0o600);
    std::fs::set_permissions(path, permissions)
        .wrap_err_with(|| format!("не удалось выставить 0600 для {}", path.display()))
}

#[cfg(not(unix))]
pub(crate) fn secure_session_file(_path: &Path) -> Result<()> {
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::{Config, SessionId};
    use std::collections::HashMap;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    #[cfg(unix)]
    use std::sync::atomic::{AtomicU64, Ordering};

    fn parse_at(values: &[(&str, &str)], cwd: &Path) -> color_eyre::Result<Config> {
        let mut env = HashMap::from([("api_id", "123"), ("api_hash", "hash")]);
        env.extend(values.iter().copied());
        Config::from_values(|key| env.get(key).map(ToString::to_string), cwd)
    }

    fn parse(values: &[(&str, &str)]) -> color_eyre::Result<Config> {
        parse_at(values, Path::new("/srv/lnl"))
    }

    #[cfg(unix)]
    struct TestDir(std::path::PathBuf);

    #[cfg(unix)]
    impl TestDir {
        fn new(label: &str) -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "lnl-{label}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    #[cfg(unix)]
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn legacy_config_keeps_existing_session_contract() {
        let config = parse(&[("TG_FOLDER", "LNL")]).unwrap();

        assert_eq!(config.default_session.as_str(), "default");
        assert_eq!(config.sessions.len(), 1);
        assert_eq!(config.sessions[0].id.as_str(), "default");
        assert_eq!(config.sessions[0].file, Path::new("/srv/lnl/lnl.session"));
        assert_eq!(config.sessions[0].folder, "LNL");
    }

    #[test]
    fn legacy_session_file_override_is_resolved_under_session_dir() {
        let config = parse(&[
            ("TG_FOLDER", "LNL"),
            ("TG_SESSION_DIR", "sessions"),
            ("TG_SESSION_FILE", "existing.session"),
        ])
        .unwrap();

        assert_eq!(
            config.sessions[0].file,
            Path::new("/srv/lnl/sessions/existing.session")
        );
    }

    #[test]
    fn multi_session_config_preserves_order_and_supports_overrides() {
        let config = parse(&[
            ("TG_SESSIONS", "personal, default, work"),
            ("TG_DEFAULT_SESSION", "work"),
            ("TG_SESSION_DIR", "sessions"),
            ("TG_FOLDER", "LNL"),
            ("TG_SESSION_WORK_FOLDER", "Support"),
            ("TG_SESSION_PERSONAL_FILE", "../personal.session"),
        ])
        .unwrap();

        assert_eq!(config.default_session.as_str(), "work");
        assert_eq!(
            config
                .sessions
                .iter()
                .map(|session| session.id.as_str())
                .collect::<Vec<_>>(),
            ["personal", "default", "work"]
        );
        assert_eq!(
            config.sessions[0].file,
            Path::new("/srv/lnl/personal.session")
        );
        assert_eq!(
            config.sessions[1].file,
            Path::new("/srv/lnl/sessions/lnl.session")
        );
        assert_eq!(
            config.sessions[2].file,
            Path::new("/srv/lnl/sessions/lnl.work.session")
        );
        assert_eq!(config.sessions[2].folder, "Support");
    }

    #[test]
    fn default_id_is_preferred_when_present() {
        let config = parse(&[("TG_SESSIONS", "work,default"), ("TG_FOLDER", "LNL")]).unwrap();
        assert_eq!(config.default_session.as_str(), "default");

        let config = parse(&[("TG_SESSIONS", "work,home"), ("TG_FOLDER", "LNL")]).unwrap();
        assert_eq!(config.default_session.as_str(), "work");
    }

    #[test]
    fn rejects_invalid_or_duplicate_session_ids() {
        for value in ["", "work,", "Work", "../work", "_work", "work,work"] {
            assert!(
                parse(&[("TG_SESSIONS", value), ("TG_FOLDER", "LNL")]).is_err(),
                "{value:?} should be rejected"
            );
        }
    }

    #[test]
    fn rejects_unknown_default_and_defaults_missing_folder_to_all() {
        assert!(
            parse(&[
                ("TG_SESSIONS", "home,work"),
                ("TG_DEFAULT_SESSION", "default"),
                ("TG_FOLDER", "LNL"),
            ])
            .is_err()
        );
        let config = parse(&[("TG_SESSIONS", "home,work")]).unwrap();
        assert_eq!(config.sessions[0].folder, "all");
        assert_eq!(config.sessions[1].folder, "all");
        let config = parse(&[("TG_FOLDER", "  ")]).unwrap();
        assert_eq!(config.sessions[0].folder, "all");
    }

    #[test]
    fn each_session_can_define_its_own_folder_without_a_global_folder() {
        let config = parse(&[
            ("TG_SESSIONS", "home,work"),
            ("TG_SESSION_HOME_FOLDER", "Личное"),
            ("TG_SESSION_WORK_FOLDER", "Support"),
        ])
        .unwrap();
        assert_eq!(config.sessions[0].folder, "Личное");
        assert_eq!(config.sessions[1].folder, "Support");
    }

    #[test]
    fn rejects_session_file_collisions() {
        assert!(
            parse(&[
                ("TG_SESSIONS", "default,work"),
                ("TG_FOLDER", "LNL"),
                ("TG_SESSION_DEFAULT_FILE", "shared.session"),
                ("TG_SESSION_WORK_FILE", "./shared.session"),
            ])
            .is_err()
        );
        assert!(
            parse(&[
                ("TG_SESSIONS", "default,work"),
                ("TG_FOLDER", "LNL"),
                ("TG_SESSION_DEFAULT_FILE", "shared.session"),
                ("TG_SESSION_WORK_FILE", "shared.bak"),
            ])
            .is_err()
        );
        assert!(
            parse(&[
                ("TG_SESSIONS", "default,work"),
                ("TG_FOLDER", "LNL"),
                ("TG_SESSION_DEFAULT_FILE", "Case.session"),
                ("TG_SESSION_WORK_FILE", "case.session"),
            ])
            .is_err()
        );
    }

    #[cfg(unix)]
    #[test]
    fn prepare_rejects_symlink_and_hardlink_aliases() {
        let symlink_dir = TestDir::new("symlink");
        let original = symlink_dir.0.join("one.session");
        let alias = symlink_dir.0.join("two.session");
        std::fs::write(&original, b"session").unwrap();
        std::os::unix::fs::symlink(&original, &alias).unwrap();
        let config = parse_at(
            &[
                ("TG_SESSIONS", "one,two"),
                ("TG_FOLDER", "LNL"),
                ("TG_SESSION_ONE_FILE", "one.session"),
                ("TG_SESSION_TWO_FILE", "two.session"),
            ],
            &symlink_dir.0,
        )
        .unwrap();
        assert!(config.prepare_session_files().is_err());

        let hardlink_dir = TestDir::new("hardlink");
        let original = hardlink_dir.0.join("one.session");
        let alias = hardlink_dir.0.join("two.session");
        std::fs::write(&original, b"session").unwrap();
        std::fs::hard_link(&original, &alias).unwrap();
        let config = parse_at(
            &[
                ("TG_SESSIONS", "one,two"),
                ("TG_FOLDER", "LNL"),
                ("TG_SESSION_ONE_FILE", "one.session"),
                ("TG_SESSION_TWO_FILE", "two.session"),
            ],
            &hardlink_dir.0,
        )
        .unwrap();
        assert!(config.prepare_session_files().is_err());
    }

    #[cfg(unix)]
    #[test]
    fn prepare_protects_existing_session_file() {
        let directory = TestDir::new("permissions");
        let file = directory.0.join("lnl.session");
        std::fs::write(&file, b"session").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();

        let config = parse_at(&[("TG_FOLDER", "LNL")], &directory.0).unwrap();
        config.prepare_session_files().unwrap();

        let mode = std::fs::metadata(file).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn session_id_serializes_as_a_string() {
        let id: SessionId = "work".parse().unwrap();
        assert_eq!(serde_json::to_string(&id).unwrap(), "\"work\"");
    }

    #[test]
    fn rejects_non_positive_api_id() {
        assert!(parse(&[("api_id", "0"), ("TG_FOLDER", "LNL")]).is_err());
        assert!(parse(&[("api_id", "-1"), ("TG_FOLDER", "LNL")]).is_err());
    }
}
