use crate::api::SessionState;
use crate::config::types::ChatKey;
use ferogram::update::Update;

use super::media::message_from_incoming;

pub async fn handle_update(state: &SessionState, update: Update) {
    match update {
        Update::NewMessage(msg) => {
            let Some(peer) = msg.peer_id() else {
                return;
            };
            let Some(key) = ChatKey::from_peer(peer) else {
                return;
            };
            let Some(message) = message_from_incoming(&msg) else {
                return;
            };
            state.record_message(key, message);
        }
        Update::Raw(raw) => {
            let ferogram::tl::enums::Update::DialogPinned(update) = raw.inner else {
                return;
            };
            if !matches!(update.folder_id.unwrap_or(0), 0 | 1) {
                return;
            }
            let ferogram::tl::enums::DialogPeer::DialogPeer(peer) = update.peer else {
                return;
            };
            let Some(key) = ChatKey::from_peer(&peer.peer) else {
                return;
            };
            state.record_chat_pinned(key, update.pinned);
        }
        _ => {}
    }
}
