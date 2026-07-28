use ferogram::Client;
use std::sync::Arc;
use tokio::sync::broadcast;

use crate::config::types::{Telegram, WsEvent};

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
}
