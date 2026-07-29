use ferogram::{Client, InputMessage};
use serde::Serialize;
use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::config::SessionId;
use crate::config::types::{ChatKey, Message, Telegram, WsEvent};

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

pub struct SessionState<C = Client> {
    id: SessionId,
    pub client: Arc<C>,
    pub telegram: Arc<Telegram>,
    pub events: broadcast::Sender<WsEvent>,
}

impl<C> SessionState<C> {
    pub fn new(id: SessionId, client: Arc<C>, telegram: Arc<Telegram>) -> Self {
        let (events, _) = broadcast::channel(256);
        Self {
            id,
            client,
            telegram,
            events,
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
}

pub struct AppState<C = Client> {
    sessions: Arc<HashMap<SessionId, Arc<SessionState<C>>>>,
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
    pub fn new(
        sessions: Vec<Arc<SessionState<C>>>,
        default_session: SessionId,
    ) -> Result<Self, String> {
        if sessions.is_empty() {
            return Err("нужна хотя бы одна Telegram-сессия".to_string());
        }

        let mut registry = HashMap::with_capacity(sessions.len());
        let mut order = Vec::with_capacity(sessions.len());
        for session in sessions {
            let id = session.id().clone();
            if registry.insert(id.clone(), session).is_some() {
                return Err(format!("повтор сессии «{id}»"));
            }
            order.push(id);
        }
        if !registry.contains_key(&default_session) {
            return Err(format!("default-сессия «{default_session}» не настроена"));
        }

        Ok(Self {
            sessions: Arc::new(registry),
            order: Arc::new(order),
            default_session,
            api_shutdown: CancellationToken::new(),
        })
    }

    pub fn session(&self, id: &str) -> Option<Arc<SessionState<C>>> {
        self.sessions.get(id).cloned()
    }

    pub fn default_session(&self) -> Arc<SessionState<C>> {
        Arc::clone(
            self.sessions
                .get(&self.default_session)
                .expect("default session is validated during construction"),
        )
    }

    pub fn summaries(&self) -> Vec<SessionSummary> {
        self.order
            .iter()
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
    use crate::config::types::{ChatKey, Dialogue, Message, Telegram};
    use std::sync::Arc;

    fn message(text: &str) -> Message {
        Message {
            id: 42,
            text: text.to_string(),
            outgoing: false,
            date: 0,
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
