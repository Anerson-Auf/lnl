use ferogram::{Client, InputMessage};
use serde::Serialize;
use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::sync::RwLock;
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore, broadcast};
use tokio_util::sync::CancellationToken;

use crate::config::SessionId;
use crate::config::types::{ChatKey, Message, Telegram, WsEvent};
use crate::tg::media::message_from_incoming;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SentMessage {
    pub id: i32,
    pub date: i32,
}

pub(crate) trait MessageSender: Send + Sync + 'static {
    fn send_text(
        &self,
        peer_id: i64,
        text: String,
    ) -> impl Future<Output = Result<SentMessage, String>> + Send;
}

pub(crate) trait HistoryLoader: Send + Sync + 'static {
    fn load_history(
        &self,
        peer_id: i64,
        limit: i32,
    ) -> impl Future<Output = Result<Vec<Message>, String>> + Send;
}

impl MessageSender for Client {
    async fn send_text(&self, peer_id: i64, text: String) -> Result<SentMessage, String> {
        let sent = self
            .send_message(peer_id, InputMessage::text(text.as_str()))
            .await
            .map_err(|error| error.to_string())?;
        Ok(SentMessage {
            id: sent.id(),
            date: sent.date(),
        })
    }
}

impl HistoryLoader for Client {
    async fn load_history(&self, peer_id: i64, limit: i32) -> Result<Vec<Message>, String> {
        let page = self
            .get_message_history(peer_id, limit, 0, 0)
            .await
            .map_err(|error| error.to_string())?;
        let mut history = page
            .messages
            .into_iter()
            .filter_map(|message| message_from_incoming(&message))
            .collect::<Vec<_>>();
        history.reverse();
        Ok(history)
    }
}

pub struct SessionState<C = Client> {
    id: SessionId,
    pub client: Arc<C>,
    pub telegram: Arc<Telegram>,
    pub events: broadcast::Sender<WsEvent>,
    history_loads: dashmap::DashMap<ChatKey, Arc<Mutex<()>>>,
    upload_lock: Mutex<()>,
    download_limit: Arc<Semaphore>,
}

impl<C> SessionState<C> {
    pub fn new(id: SessionId, client: Arc<C>, telegram: Arc<Telegram>) -> Self {
        let (events, _) = broadcast::channel(256);
        Self {
            id,
            client,
            telegram,
            events,
            history_loads: dashmap::DashMap::new(),
            upload_lock: Mutex::new(()),
            download_limit: Arc::new(Semaphore::new(3)),
        }
    }

    pub fn id(&self) -> &SessionId {
        &self.id
    }

    pub fn record_message(&self, key: ChatKey, message: Message) -> bool {
        let Some(mut dialogue) = self.telegram.dialogues.get_mut(&key) else {
            return false;
        };
        if !dialogue.insert_new_message(message.clone()) {
            return false;
        }

        let _ = self.events.send(WsEvent::NewMessage {
            peer_id: key.bot_api_id(),
            message,
        });
        true
    }

    pub fn record_chat_pinned(&self, key: ChatKey, pinned: bool) -> bool {
        let Some(mut dialogue) = self.telegram.dialogues.get_mut(&key) else {
            return false;
        };
        if dialogue.pinned == Some(pinned) {
            return false;
        }
        dialogue.pinned = Some(pinned);
        let _ = self.events.send(WsEvent::ChatPinned {
            peer_id: key.bot_api_id(),
            pinned,
        });
        true
    }

    pub async fn lock_upload(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.upload_lock.lock().await
    }

    pub fn try_download_permit(&self) -> Option<OwnedSemaphorePermit> {
        Arc::clone(&self.download_limit).try_acquire_owned().ok()
    }

    pub async fn history(&self, key: ChatKey) -> Result<Option<Vec<Message>>, String>
    where
        C: HistoryLoader,
    {
        let Some(dialogue) = self.telegram.dialogues.get(&key) else {
            return Ok(None);
        };
        if dialogue.history_loaded {
            return Ok(Some(dialogue.history.clone()));
        }
        drop(dialogue);

        let load = self
            .history_loads
            .entry(key)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone();
        let _guard = load.lock().await;

        let Some(dialogue) = self.telegram.dialogues.get(&key) else {
            return Ok(None);
        };
        if dialogue.history_loaded {
            return Ok(Some(dialogue.history.clone()));
        }
        drop(dialogue);

        let messages = self.client.load_history(key.bot_api_id(), 30).await?;
        let Some(mut dialogue) = self.telegram.dialogues.get_mut(&key) else {
            return Ok(None);
        };
        for message in messages {
            dialogue.insert_new_message(message);
        }
        dialogue.history_loaded = true;
        Ok(Some(dialogue.history.clone()))
    }
}

pub struct AppState<C = Client> {
    sessions: Arc<RwLock<HashMap<SessionId, Arc<SessionState<C>>>>>,
    order: Arc<Vec<SessionId>>,
    default_session: SessionId,
    api_shutdown: CancellationToken,
}

impl<C> Clone for AppState<C> {
    fn clone(&self) -> Self {
        Self {
            sessions: Arc::clone(&self.sessions),
            order: Arc::clone(&self.order),
            default_session: self.default_session.clone(),
            api_shutdown: self.api_shutdown.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SessionSummary {
    pub id: SessionId,
    pub is_default: bool,
}

impl<C> AppState<C> {
    #[cfg(test)]
    pub fn new(
        sessions: Vec<Arc<SessionState<C>>>,
        default_session: SessionId,
    ) -> Result<Self, String> {
        let mut registry = HashMap::with_capacity(sessions.len());
        let mut order = Vec::with_capacity(sessions.len());
        for session in sessions {
            let id = session.id().clone();
            if registry.insert(id.clone(), session).is_some() {
                return Err(format!("повтор сессии «{id}»"));
            }
            order.push(id);
        }
        if !registry.is_empty() && !registry.contains_key(&default_session) {
            return Err(format!("default-сессия «{default_session}» не настроена"));
        }

        Ok(Self {
            sessions: Arc::new(RwLock::new(registry)),
            order: Arc::new(order),
            default_session,
            api_shutdown: CancellationToken::new(),
        })
    }

    pub fn with_order(
        sessions: Vec<Arc<SessionState<C>>>,
        order: Vec<SessionId>,
        default_session: SessionId,
    ) -> Result<Self, String> {
        let mut seen = std::collections::HashSet::with_capacity(order.len());
        if order.iter().any(|id| !seen.insert(id.clone())) {
            return Err("повтор id в порядке Telegram-сессий".to_string());
        }
        if !order.contains(&default_session) {
            return Err(format!("default-сессия «{default_session}» не настроена"));
        }

        let state = Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            order: Arc::new(order),
            default_session,
            api_shutdown: CancellationToken::new(),
        };
        for session in sessions {
            state.insert_session(session)?;
        }
        Ok(state)
    }

    pub fn session(&self, id: &str) -> Option<Arc<SessionState<C>>> {
        self.sessions
            .read()
            .expect("session registry lock poisoned")
            .get(id)
            .cloned()
    }

    pub fn is_configured(&self, id: &str) -> bool {
        self.order
            .iter()
            .any(|configured| configured.as_str() == id)
    }

    pub fn default_session(&self) -> Option<Arc<SessionState<C>>> {
        self.session(self.default_session.as_str())
    }

    pub fn insert_session(&self, session: Arc<SessionState<C>>) -> Result<(), String> {
        let id = session.id().clone();
        if !self.order.contains(&id) {
            return Err(format!("сессия «{id}» отсутствует в конфигурации"));
        }
        let mut sessions = self
            .sessions
            .write()
            .map_err(|_| "session registry lock poisoned".to_string())?;
        if sessions.contains_key(&id) {
            return Err(format!("сессия «{id}» уже готова"));
        }
        sessions.insert(id, session);
        Ok(())
    }

    pub fn summaries(&self) -> Vec<SessionSummary> {
        let sessions = self
            .sessions
            .read()
            .expect("session registry lock poisoned");
        self.order
            .iter()
            .filter(|id| sessions.contains_key(*id))
            .map(|id| SessionSummary {
                id: id.clone(),
                is_default: id == &self.default_session,
            })
            .collect()
    }

    pub fn api_shutdown(&self) -> CancellationToken {
        self.api_shutdown.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::{AppState, SessionState};
    use crate::config::types::{ChatKey, Dialogue, Message, Telegram, WsEvent};
    use std::sync::Arc;

    fn message(text: &str) -> Message {
        Message {
            id: 42,
            text: text.to_string(),
            outgoing: false,
            date: 0,
            media: None,
        }
    }

    #[test]
    fn recording_is_isolated_between_sessions() {
        let first_telegram = Arc::new(Telegram {
            dialogues: [(
                ChatKey::User(1),
                Dialogue {
                    title: "first".to_string(),
                    history: Vec::new(),
                    history_loaded: true,
                    pinned: None,
                },
            )]
            .into_iter()
            .collect(),
        });
        let second_telegram = Arc::new(Telegram {
            dialogues: [(
                ChatKey::User(1),
                Dialogue {
                    title: "second".to_string(),
                    history: Vec::new(),
                    history_loaded: true,
                    pinned: None,
                },
            )]
            .into_iter()
            .collect(),
        });
        let first = SessionState::new(
            "first".parse().unwrap(),
            Arc::new(()),
            Arc::clone(&first_telegram),
        );
        let second = SessionState::new(
            "second".parse().unwrap(),
            Arc::new(()),
            Arc::clone(&second_telegram),
        );
        let mut first_events = first.events.subscribe();
        let mut second_events = second.events.subscribe();

        assert!(first.record_message(ChatKey::User(1), message("only first")));
        assert_eq!(
            first_telegram
                .dialogues
                .get(&ChatKey::User(1))
                .unwrap()
                .history[0]
                .text,
            "only first"
        );
        assert!(
            second_telegram
                .dialogues
                .get(&ChatKey::User(1))
                .unwrap()
                .history
                .is_empty()
        );
        assert!(first_events.try_recv().is_ok());
        assert!(second_events.try_recv().is_err());

        assert!(!first.record_message(ChatKey::User(1), message("duplicate")));
        assert!(first_events.try_recv().is_err());

        assert!(first.record_chat_pinned(ChatKey::User(1), true));
        assert_eq!(
            first_telegram
                .dialogues
                .get(&ChatKey::User(1))
                .unwrap()
                .pinned,
            Some(true)
        );
        assert!(matches!(
            first_events.try_recv(),
            Ok(WsEvent::ChatPinned {
                peer_id: 1,
                pinned: true
            })
        ));
        assert!(!first.record_chat_pinned(ChatKey::User(1), true));
        assert!(first_events.try_recv().is_err());
        assert!(second_events.try_recv().is_err());
    }

    #[test]
    fn registry_rejects_duplicate_and_unknown_default_sessions() {
        let telegram = Arc::new(Telegram {
            dialogues: Default::default(),
        });
        let session = Arc::new(SessionState::new(
            "home".parse().unwrap(),
            Arc::new(()),
            telegram,
        ));

        assert!(
            AppState::new(
                vec![Arc::clone(&session), Arc::clone(&session)],
                "home".parse().unwrap(),
            )
            .is_err()
        );
        assert!(AppState::new(vec![session], "work".parse().unwrap()).is_err());
    }
}
