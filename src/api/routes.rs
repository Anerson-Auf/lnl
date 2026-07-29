use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::config::types::{ChatKey, ChatSummary, Message};

use super::state::{AppState, MessageSender, SessionState};

#[derive(Serialize)]
pub(crate) struct ErrorBody {
    pub error: String,
}

pub(crate) fn err(status: StatusCode, msg: impl Into<String>) -> (StatusCode, Json<ErrorBody>) {
    (status, Json(ErrorBody { error: msg.into() }))
}

type ApiError = (StatusCode, Json<ErrorBody>);

pub async fn list_sessions<C: Send + Sync + 'static>(
    State(state): State<AppState<C>>,
) -> impl IntoResponse {
    Json(state.summaries())
}

pub async fn list_default_chats<C: Send + Sync + 'static>(
    State(state): State<AppState<C>>,
) -> impl IntoResponse {
    list_chats_for(state.default_session())
}

pub async fn list_session_chats<C: Send + Sync + 'static>(
    State(state): State<AppState<C>>,
    Path(session_id): Path<String>,
) -> Result<Json<Vec<ChatSummary>>, ApiError> {
    Ok(list_chats_for(resolve_session(&state, &session_id)?))
}

fn list_chats_for<C>(session: Arc<SessionState<C>>) -> Json<Vec<ChatSummary>> {
    let mut chats: Vec<ChatSummary> = session
        .telegram
        .dialogues
        .iter()
        .map(|entry| {
            let key = *entry.key();
            let dialogue = entry.value();
            ChatSummary {
                peer_id: key.bot_api_id(),
                title: dialogue.title.clone(),
                last_message: dialogue.history.last().map(|message| message.text.clone()),
            }
        })
        .collect();
    chats.sort_by_key(|chat| chat.title.to_lowercase());
    Json(chats)
}

pub async fn get_default_messages<C: Send + Sync + 'static>(
    State(state): State<AppState<C>>,
    Path(peer_id): Path<i64>,
) -> Result<Json<Vec<Message>>, ApiError> {
    get_messages_for(state.default_session(), peer_id)
}

pub async fn get_session_messages<C: Send + Sync + 'static>(
    State(state): State<AppState<C>>,
    Path((session_id, peer_id)): Path<(String, i64)>,
) -> Result<Json<Vec<Message>>, ApiError> {
    get_messages_for(resolve_session(&state, &session_id)?, peer_id)
}

fn get_messages_for<C>(
    session: Arc<SessionState<C>>,
    peer_id: i64,
) -> Result<Json<Vec<Message>>, ApiError> {
    let key = ChatKey::from_bot_api_id(peer_id);
    let Some(dialogue) = session.telegram.dialogues.get(&key) else {
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

pub async fn send_default_message<C: MessageSender>(
    State(state): State<AppState<C>>,
    Path(peer_id): Path<i64>,
    Json(body): Json<SendBody>,
) -> Result<Json<SendResponse>, ApiError> {
    send_message_for(state.default_session(), peer_id, body).await
}

pub async fn send_session_message<C: MessageSender>(
    State(state): State<AppState<C>>,
    Path((session_id, peer_id)): Path<(String, i64)>,
    Json(body): Json<SendBody>,
) -> Result<Json<SendResponse>, ApiError> {
    send_message_for(resolve_session(&state, &session_id)?, peer_id, body).await
}

async fn send_message_for<C: MessageSender>(
    session: Arc<SessionState<C>>,
    peer_id: i64,
    body: SendBody,
) -> Result<Json<SendResponse>, ApiError> {
    let text = body.text;
    if text.trim().is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "пустой text"));
    }

    let key = ChatKey::from_bot_api_id(peer_id);
    if !session.telegram.dialogues.contains_key(&key) {
        return Err(err(
            StatusCode::NOT_FOUND,
            format!("чат {peer_id} не в папке релея"),
        ));
    }

    let sent = session
        .client
        .send_text(peer_id, text.clone())
        .await
        .map_err(|error| err(StatusCode::BAD_GATEWAY, format!("telegram: {error}")))?;
    let message = Message {
        id: sent.id,
        text,
        outgoing: true,
        date: sent.date,
    };
    session.record_message(key, message.clone());

    Ok(Json(SendResponse { ok: true, message }))
}

fn resolve_session<C>(
    state: &AppState<C>,
    session_id: &str,
) -> Result<Arc<SessionState<C>>, ApiError> {
    state
        .session(session_id)
        .ok_or_else(|| err(StatusCode::NOT_FOUND, format!("нет сессии {session_id}")))
}
