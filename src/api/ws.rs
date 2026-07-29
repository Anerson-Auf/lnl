use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::config::types::WsEvent;

use super::routes::err;
use super::state::AppState;

pub async fn default_ws_handler<C>(
    ws: WebSocketUpgrade,
    State(state): State<AppState<C>>,
) -> Response
where
    C: Send + Sync + 'static,
{
    let Some(session) = state.default_session() else {
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            "основной аккаунт ещё не авторизован",
        )
        .into_response();
    };
    upgrade(ws, session.events.subscribe(), state.api_shutdown())
}

pub async fn session_ws_handler<C>(
    ws: WebSocketUpgrade,
    State(state): State<AppState<C>>,
    Path(session_id): Path<String>,
) -> Response
where
    C: Send + Sync + 'static,
{
    let Some(session) = state.session(&session_id) else {
        let (status, message) = if state.is_configured(&session_id) {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                format!("аккаунт {session_id} ещё не авторизован"),
            )
        } else {
            (StatusCode::NOT_FOUND, format!("нет сессии {session_id}"))
        };
        return err(status, message).into_response();
    };
    upgrade(ws, session.events.subscribe(), state.api_shutdown())
}

fn upgrade(
    ws: WebSocketUpgrade,
    events: broadcast::Receiver<WsEvent>,
    shutdown: CancellationToken,
) -> Response {
    ws.on_upgrade(move |socket| handle_socket(socket, events, shutdown))
        .into_response()
}

async fn handle_socket(
    socket: WebSocket,
    mut events: broadcast::Receiver<WsEvent>,
    shutdown: CancellationToken,
) {
    let (mut sink, mut stream) = socket.split();

    let forward = async {
        loop {
            match events.recv().await {
                Ok(event) => {
                    let Ok(json) = serde_json::to_string(&event) else {
                        continue;
                    };
                    if sink.send(WsMessage::Text(json.into())).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    };

    let read = async {
        while let Some(Ok(message)) = stream.next().await {
            if matches!(message, WsMessage::Close(_)) {
                break;
            }
        }
    };

    tokio::select! {
        _ = forward => {},
        _ = read => {},
        _ = shutdown.cancelled() => {},
    }
}
