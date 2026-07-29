use dashmap::DashMap;
use ferogram::tl;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    pub id: i32,
    pub text: String,
    pub outgoing: bool,
    pub date: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<MediaInfo>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Sticker,
    Photo,
    File,
    Audio,
    Video,
    Voice,
    VideoNote,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StickerFormat {
    Static,
    Animated,
    Video,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MediaInfo {
    pub kind: MediaKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sticker_format: Option<StickerFormat>,
    pub downloadable: bool,
    pub spoiler: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Dialogue {
    pub title: String,
    pub history: Vec<Message>,
    pub history_loaded: bool,
    pub pinned: Option<bool>,
}

impl Dialogue {
    pub fn insert_new_message(&mut self, message: Message) -> bool {
        match self
            .history
            .binary_search_by_key(&message.id, |existing| existing.id)
        {
            Ok(_) => false,
            Err(index) => {
                self.history.insert(index, message);
                true
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatKey {
    User(i64),
    Chat(i64),
    Channel(i64),
}

impl ChatKey {
    pub fn from_peer(peer: &tl::enums::Peer) -> Option<Self> {
        match peer {
            tl::enums::Peer::User(u) if u.user_id != 0 => Some(Self::User(u.user_id)),
            tl::enums::Peer::Chat(c) => Some(Self::Chat(c.chat_id)),
            tl::enums::Peer::Channel(c) => Some(Self::Channel(c.channel_id)),
            _ => None,
        }
    }

    pub fn from_bot_api_id(id: i64) -> Self {
        const ZERO_CHANNEL_ID: i64 = -1_000_000_000_000;
        if id > 0 {
            Self::User(id)
        } else if id <= ZERO_CHANNEL_ID {
            Self::Channel(-(id + 1_000_000_000_000))
        } else {
            Self::Chat(-id)
        }
    }

    pub fn bot_api_id(self) -> i64 {
        match self {
            Self::User(id) => id,
            Self::Chat(id) => -id,
            Self::Channel(id) => -(id + 1_000_000_000_000),
        }
    }
}

pub struct Telegram {
    pub dialogues: DashMap<ChatKey, Dialogue>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ChatSummary {
    pub peer_id: i64,
    pub title: String,
    pub last_message: Option<String>,
    #[serde(skip_serializing_if = "not_pinned")]
    pub pinned: Option<bool>,
}

fn not_pinned(value: &Option<bool>) -> bool {
    *value != Some(true)
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsEvent {
    NewMessage { peer_id: i64, message: Message },
    ChatPinned { peer_id: i64, pinned: bool },
}

#[cfg(test)]
mod tests {
    use super::{Dialogue, Message};

    fn message(id: i32) -> Message {
        Message {
            id,
            text: "text".to_string(),
            outgoing: false,
            date: 0,
            media: None,
        }
    }

    #[test]
    fn insert_new_message_deduplicates_and_orders_by_id() {
        let mut dialogue = Dialogue {
            title: "chat".to_string(),
            history: Vec::new(),
            history_loaded: true,
            pinned: None,
        };

        assert!(dialogue.insert_new_message(message(43)));
        assert!(dialogue.insert_new_message(message(42)));
        assert!(!dialogue.insert_new_message(message(42)));
        assert_eq!(
            dialogue
                .history
                .iter()
                .map(|message| message.id)
                .collect::<Vec<_>>(),
            [42, 43]
        );
    }

    #[test]
    fn text_message_json_keeps_the_legacy_shape() {
        let value = serde_json::to_value(message(42)).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "id": 42,
                "text": "text",
                "outgoing": false,
                "date": 0
            })
        );
    }
}
