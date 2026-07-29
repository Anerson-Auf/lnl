use ferogram::update::Update;
use std::sync::Arc;

use crate::api::AppState;
use crate::config::types::{ChatKey, Message, WsEvent};

pub async fn handle_update(state: Arc<AppState>, update: Update) {
    let Update::NewMessage(msg) = update else {
        return;
    };

    let Some(text) = msg.text() else {
        return;
    };
    if text.trim().is_empty() {
        return;
    }

    let Some(peer) = msg.peer_id() else {
        return;
    };
    let Some(key) = ChatKey::from_peer(peer) else {
        return;
    };

    // Только чаты из папки релея.
    if !state.telegram.dialogues.contains_key(&key) {
        return;
    }

    let message = Message {
        id: msg.id(),
        text: text.to_string(),
        outgoing: msg.outgoing(),
        date: msg.date(),
    };
    let peer_id = key.bot_api_id();

    let inserted = state.insert_message(key, message.clone());

    if inserted {
        let _ = state.events.send(WsEvent::NewMessage { peer_id, message });
    }
}
