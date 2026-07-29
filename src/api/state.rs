use ferogram::Client;
use std::sync::Arc;
use tokio::sync::broadcast;

use crate::config::types::{ChatKey, Message, Telegram, WsEvent};

#[derive(Clone)]
pub struct AppState {
    pub client: Arc<Client>,
    pub telegram: Arc<Telegram>,
    pub events: broadcast::Sender<WsEvent>,
}

impl AppState {
    pub fn new(client: Arc<Client>, telegram: Arc<Telegram>) -> Self {
        let (events, _) = broadcast::channel(256);
        Self {
            client,
            telegram,
            events,
        }
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
