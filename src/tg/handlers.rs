use ferogram::update::Update;
use std::sync::Arc;

use crate::api::AppState;
use crate::config::types::{ChatKey, Message, WsEvent};

pub async fn handle_update(state: Arc<AppState>, update: Update) {
    let Update::NewMessage(msg) = update else {
        return;
    };

    let text = msg.text().unwrap_or("").trim();
    if text.is_empty() {
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

    if let Some(mut dialogue) = state.telegram.dialogues.get_mut(&key) {
        if !dialogue.history.iter().any(|m| m.id == message.id) {
            dialogue.history.push(message.clone());
        }
    }

    // println!(
    //     "{} {}: {}",
    //     if message.outgoing { "→" } else { "←" },
    //     peer_id,
    //     message.text
    // );

    let _ = state.events.send(WsEvent::NewMessage { peer_id, message });
}
