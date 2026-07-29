mod routes;
mod state;
mod ws;

use std::net::SocketAddr;
use std::path::PathBuf;

use axum::Router;
use axum::routing::get;
use color_eyre::Result;
use ferogram::Client;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;

use state::MessageSender;
pub use state::{AppState, SessionState};

fn api_router<C>(state: AppState<C>) -> Router
where
    C: MessageSender,
{
    Router::new()
        .route("/api/sessions", get(routes::list_sessions::<C>))
        .route(
            "/api/sessions/{session_id}/chats",
            get(routes::list_session_chats::<C>),
        )
        .route(
            "/api/sessions/{session_id}/messages/{peer_id}",
            get(routes::get_session_messages::<C>).post(routes::send_session_message::<C>),
        )
        .route(
            "/api/sessions/{session_id}/ws",
            get(ws::session_ws_handler::<C>),
        )
        .route("/api/chats", get(routes::list_default_chats::<C>))
        .route(
            "/api/messages/{peer_id}",
            get(routes::get_default_messages::<C>).post(routes::send_default_message::<C>),
        )
        .route("/ws", get(ws::default_ws_handler::<C>))
        .with_state(state)
}

pub async fn serve(
    state: AppState<Client>,
    listener: tokio::net::TcpListener,
    bind: SocketAddr,
) -> Result<()> {
    let shutdown = state.api_shutdown();
    let api = api_router(state);

    let app = if std::env::var("LNL_DEBUG_UI").ok().as_deref() == Some("1") {
        let static_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("static");
        println!("Debug UI: http://{bind}/  (LNL_DEBUG_UI=1)");
        Router::new()
            .merge(api)
            .fallback_service(ServeDir::new(static_dir).append_index_html_on_directories(true))
            .layer(CorsLayer::permissive())
    } else {
        println!("API: http://{bind}/api/…  WS: ws://{bind}/ws  (клиент = Android)");
        api.layer(CorsLayer::permissive())
    };

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown.cancelled_owned())
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::state::{MessageSender, SentMessage};
    use super::{AppState, SessionState, api_router};
    use crate::config::types::{ChatKey, Dialogue, Message, Telegram};
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use serde_json::Value;
    use std::sync::{Arc, Mutex};
    use tower::ServiceExt;

    #[derive(Default)]
    struct FakeClient {
        sent: Mutex<Vec<(i64, String)>>,
        fail: bool,
    }

    impl FakeClient {
        fn failing() -> Self {
            Self {
                sent: Mutex::new(Vec::new()),
                fail: true,
            }
        }
    }

    impl MessageSender for FakeClient {
        async fn send_text(&self, peer_id: i64, text: String) -> Result<SentMessage, String> {
            if self.fail {
                return Err("offline".to_string());
            }
            self.sent.lock().unwrap().push((peer_id, text));
            Ok(SentMessage { id: 99, date: 7 })
        }
    }

    fn telegram(title: &str, text: &str) -> Arc<Telegram> {
        Arc::new(Telegram {
            dialogues: [(
                ChatKey::User(1),
                Dialogue {
                    title: title.to_string(),
                    history: vec![Message {
                        id: 1,
                        text: text.to_string(),
                        outgoing: false,
                        date: 1,
                    }],
                },
            )]
            .into_iter()
            .collect(),
        })
    }

    fn state() -> (AppState<FakeClient>, Arc<FakeClient>, Arc<FakeClient>) {
        let default_client = Arc::new(FakeClient::default());
        let work_client = Arc::new(FakeClient::default());
        let sessions = vec![
            Arc::new(SessionState::new(
                "default".parse().unwrap(),
                Arc::clone(&default_client),
                telegram("Home", "home"),
            )),
            Arc::new(SessionState::new(
                "work".parse().unwrap(),
                Arc::clone(&work_client),
                telegram("Work", "work"),
            )),
        ];
        (
            AppState::new(sessions, "default".parse().unwrap()).unwrap(),
            default_client,
            work_client,
        )
    }

    async fn body(response: axum::response::Response) -> Vec<u8> {
        to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec()
    }

    #[tokio::test]
    async fn legacy_routes_match_the_default_scoped_session() {
        let (state, _, _) = state();
        let app = api_router(state);
        for (legacy, scoped) in [
            ("/api/chats", "/api/sessions/default/chats"),
            ("/api/messages/1", "/api/sessions/default/messages/1"),
        ] {
            let legacy = app
                .clone()
                .oneshot(Request::get(legacy).body(Body::empty()).unwrap())
                .await
                .unwrap();
            let scoped = app
                .clone()
                .oneshot(Request::get(scoped).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(legacy.status(), scoped.status());
            assert_eq!(body(legacy).await, body(scoped).await);
        }
    }

    #[tokio::test]
    async fn scoped_routes_never_fall_back_to_default() {
        let (state, _, _) = state();
        let app = api_router(state);
        let missing = app
            .clone()
            .oneshot(
                Request::get("/api/sessions/missing/messages/1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);

        let work = app
            .oneshot(
                Request::get("/api/sessions/work/messages/1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let messages: Value = serde_json::from_slice(&body(work).await).unwrap();
        assert_eq!(messages[0]["text"], "work");
    }

    #[tokio::test]
    async fn post_uses_only_the_selected_session() {
        let (state, default_client, work_client) = state();
        let work_session = state.session("work").unwrap();
        let mut work_events = work_session.events.subscribe();
        let app = api_router(state);
        let response = app
            .oneshot(
                Request::post("/api/sessions/work/messages/1")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"text":" exact text "}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(default_client.sent.lock().unwrap().is_empty());
        assert_eq!(
            *work_client.sent.lock().unwrap(),
            [(1, " exact text ".to_string())]
        );
        assert!(work_events.try_recv().is_ok());
        assert_eq!(
            work_session
                .telegram
                .dialogues
                .get(&ChatKey::User(1))
                .unwrap()
                .history
                .last()
                .unwrap()
                .text,
            " exact text "
        );
    }

    #[tokio::test]
    async fn legacy_post_uses_only_the_default_session() {
        let (state, default_client, work_client) = state();
        let response = api_router(state)
            .oneshot(
                Request::post("/api/messages/1")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"text":"legacy"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            *default_client.sent.lock().unwrap(),
            [(1, "legacy".to_string())]
        );
        assert!(work_client.sent.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn telegram_failure_does_not_record_or_broadcast() {
        let client = Arc::new(FakeClient::failing());
        let telegram = telegram("Work", "before");
        let session = Arc::new(SessionState::new(
            "work".parse().unwrap(),
            Arc::clone(&client),
            Arc::clone(&telegram),
        ));
        let mut events = session.events.subscribe();
        let state = AppState::new(vec![session], "work".parse().unwrap()).unwrap();
        let response = api_router(state)
            .oneshot(
                Request::post("/api/sessions/work/messages/1")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"text":"after"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        assert_eq!(
            telegram
                .dialogues
                .get(&ChatKey::User(1))
                .unwrap()
                .history
                .len(),
            1
        );
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn invalid_posts_never_call_any_telegram_client() {
        let (state, default_client, work_client) = state();
        state
            .session("work")
            .unwrap()
            .telegram
            .dialogues
            .remove(&ChatKey::User(1));
        let app = api_router(state);

        let empty = app
            .clone()
            .oneshot(
                Request::post("/api/sessions/default/messages/1")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"text":"   "}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(empty.status(), StatusCode::BAD_REQUEST);

        let wrong_session = app
            .oneshot(
                Request::post("/api/sessions/work/messages/1")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"text":"message"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(wrong_session.status(), StatusCode::NOT_FOUND);
        assert!(default_client.sent.lock().unwrap().is_empty());
        assert!(work_client.sent.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn session_inventory_is_ordered_and_hides_internal_config() {
        let (state, _, _) = state();
        let response = api_router(state)
            .oneshot(Request::get("/api/sessions").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let json: Value = serde_json::from_slice(&body(response).await).unwrap();
        assert_eq!(
            json,
            serde_json::json!([
                {"id": "default", "is_default": true},
                {"id": "work", "is_default": false}
            ])
        );
    }
}
