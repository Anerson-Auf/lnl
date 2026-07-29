use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use ferogram::InputMessage;
use serde::{Deserialize, Serialize};

use crate::config::types::{ChatKey, ChatSummary, Message};

use super::state::AppState;

#[derive(Serialize)]
pub(crate) struct ErrorBody {
    pub error: String,
}

fn err(status: StatusCode, msg: impl Into<String>) -> (StatusCode, Json<ErrorBody>) {
    (
        status,
        Json(ErrorBody {
            error: msg.into(),
        }),
    )
}

pub async fn list_chats(State(state): State<AppState>) -> impl IntoResponse {
    let mut chats: Vec<ChatSummary> = state
        .telegram
        .dialogues
        .iter()
        .map(|e| {
            let key = *e.key();
            let d = e.value();
            ChatSummary {
                peer_id: key.bot_api_id(),
                title: d.title.clone(),
                last_message: d.history.last().map(|m| m.text.clone()),
            }
        })
        .collect();
    chats.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
    Json(chats)
}

pub async fn get_messages(
    State(state): State<AppState>,
    Path(peer_id): Path<i64>,
) -> Result<Json<Vec<Message>>, (StatusCode, Json<ErrorBody>)> {
    let key = ChatKey::from_bot_api_id(peer_id);
    let Some(dialogue) = state.telegram.dialogues.get(&key) else {
        return Err(err(StatusCode::NOT_FOUND, format!("нет чата {peer_id}")));
    };
    Ok(Json(dialogue.history.clone()))
}

#[derive(Deserialize)]
pub struct SendBody {
    pub text: String,
}

#[derive(Serialize)]
pub struct SendResponse {
    pub ok: bool,
    pub message: Message,
}

pub async fn send_message(
    State(state): State<AppState>,
    Path(peer_id): Path<i64>,
    Json(body): Json<SendBody>,
) -> Result<Json<SendResponse>, (StatusCode, Json<ErrorBody>)> {
    let text = body.text;
    if text.trim().is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "пустой text"));
    }

    let key = ChatKey::from_bot_api_id(peer_id);
    if !state.telegram.dialogues.contains_key(&key) {
        return Err(err(
            StatusCode::NOT_FOUND,
            format!("чат {peer_id} не в папке релея"),
        ));
    }

    let sent = state
        .client
        .send_message(peer_id, InputMessage::text(text.as_str()))
        .await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("telegram: {e}")))?;

    let message = Message {
        id: sent.id(),
        text,
        outgoing: true,
        date: sent.date(),
    };

    state.record_message(key, message.clone());

    Ok(Json(SendResponse {
        ok: true,
        message,
    }))
}
