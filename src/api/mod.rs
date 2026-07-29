mod routes;
mod state;
mod ws;

use std::net::SocketAddr;
use std::path::PathBuf;

use axum::routing::{get, post};
use axum::Router;
use color_eyre::Result;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;

pub use state::AppState;

pub async fn serve(
    state: AppState,
    listener: tokio::net::TcpListener,
    bind: SocketAddr,
) -> Result<()> {
    let api = Router::new()
        .route("/api/chats", get(routes::list_chats))
        .route("/api/messages/{peer_id}", get(routes::get_messages))
        .route("/api/messages/{peer_id}", post(routes::send_message))
        .route("/ws", get(ws::ws_handler))
        .with_state(state);

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

    axum::serve(listener, app).await?;
    Ok(())
}
