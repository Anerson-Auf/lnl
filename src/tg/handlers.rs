use ferogram::update::Update;
use std::sync::Arc;

use crate::api::AppState;
use crate::config::types::{ChatKey, Message};

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

    let message = Message {
        id: msg.id(),
        text: text.to_string(),
        outgoing: msg.outgoing(),
        date: msg.date(),
    };
    state.record_message(key, message);
}
