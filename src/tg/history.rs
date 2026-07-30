use color_eyre::{Result, eyre::eyre};
use ferogram::Client;
use ferogram::tl;
use std::collections::HashSet;

use crate::config::types::{ChatKey, Dialogue, Message, Telegram};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DialogSeed {
    Folder,
    All,
}

fn text_of(title: &tl::enums::TextWithEntities) -> &str {
    match title {
        tl::enums::TextWithEntities::TextWithEntities(t) => t.text.as_str(),
    }
}

fn same_folder_name(left: &str, right: &str) -> bool {
    left.to_lowercase() == right.to_lowercase()
}

fn chat_key_from_input(peer: &tl::enums::InputPeer) -> Option<ChatKey> {
    match peer {
        tl::enums::InputPeer::User(u) => Some(ChatKey::User(u.user_id)),
        tl::enums::InputPeer::Chat(c) => Some(ChatKey::Chat(c.chat_id)),
        tl::enums::InputPeer::Channel(c) => Some(ChatKey::Channel(c.channel_id)),
        tl::enums::InputPeer::UserFromMessage(u) => Some(ChatKey::User(u.user_id)),
        tl::enums::InputPeer::ChannelFromMessage(c) => Some(ChatKey::Channel(c.channel_id)),
        tl::enums::InputPeer::PeerSelf | tl::enums::InputPeer::Empty => None,
    }
}

fn peers_from_filter(f: &tl::enums::DialogFilter) -> Vec<tl::enums::InputPeer> {
    match f {
        tl::enums::DialogFilter::DialogFilter(f) => {
            let mut peers = Vec::with_capacity(f.pinned_peers.len() + f.include_peers.len());
            peers.extend(f.pinned_peers.iter().cloned());
            peers.extend(f.include_peers.iter().cloned());
            peers
        }
        tl::enums::DialogFilter::Chatlist(f) => {
            let mut peers = Vec::with_capacity(f.pinned_peers.len() + f.include_peers.len());
            peers.extend(f.pinned_peers.iter().cloned());
            peers.extend(f.include_peers.iter().cloned());
            peers
        }
        tl::enums::DialogFilter::Default => Vec::new(),
    }
}

async fn peer_title(client: &Client, _input: &tl::enums::InputPeer, key: ChatKey) -> String {
    if let Ok(tl::enums::Peer::User(peer)) = ferogram::PeerRef::from(key.bot_api_id())
        .resolve(client)
        .await
        && let Ok(users) = client.get_users_by_id(&[peer.user_id]).await
        && let Some(Some(user)) = users.into_iter().next()
    {
        let first = user.first_name().unwrap_or("");
        let last = user.last_name().unwrap_or("");
        let name = format!("{first} {last}").trim().to_string();
        if !name.is_empty() {
            return name;
        }
        if let Some(username) = user.username() {
            return format!("@{username}");
        }
    }
    format!("{}", key.bot_api_id())
}

pub async fn seed_dialogues_or_all(
    client: &Client,
    telegram: &Telegram,
    folder_name: &str,
    per_chat: i32,
) -> Result<DialogSeed> {
    let folder_name = folder_name.trim();
    let resp = client
        .invoke(&tl::functions::messages::GetDialogFilters {})
        .await
        .map_err(|e| eyre!("{e}"))?;
    let tl::enums::messages::DialogFilters::DialogFilters(df) = resp;

    let peers = df
        .filters
        .into_iter()
        .find(|filter| match filter {
            tl::enums::DialogFilter::DialogFilter(filter) => {
                same_folder_name(text_of(&filter.title), folder_name)
            }
            tl::enums::DialogFilter::Chatlist(filter) => {
                same_folder_name(text_of(&filter.title), folder_name)
            }
            tl::enums::DialogFilter::Default => {
                same_folder_name(folder_name, "all chats") || same_folder_name(folder_name, "all")
            }
        })
        .map(|filter| peers_from_filter(&filter))
        .unwrap_or_default();

    if peers.is_empty() {
        seed_all_dialogues(client, telegram).await?;
        return Ok(DialogSeed::All);
    }

    seed_dialogues_from_peers(client, telegram, peers, per_chat).await?;
    Ok(DialogSeed::Folder)
}

async fn seed_dialogues_from_peers(
    client: &Client,
    telegram: &Telegram,
    peers: Vec<tl::enums::InputPeer>,
    per_chat: i32,
) -> Result<()> {
    let mut seen = HashSet::new();
    for input in peers {
        let Some(key) = chat_key_from_input(&input) else {
            continue;
        };
        if !seen.insert(key) {
            continue;
        }

        let page = client
            .get_message_history(input.clone(), per_chat, 0, 0)
            .await
            .map_err(|e| eyre!("история {key:?}: {e}"))?;
        let mut history = page
            .messages
            .into_iter()
            .filter_map(|message| {
                let text = message.text()?.trim();
                (!text.is_empty()).then(|| Message {
                    id: message.id(),
                    text: text.to_string(),
                    outgoing: message.outgoing(),
                    date: message.date(),
                })
            })
            .collect::<Vec<_>>();
        history.reverse();

        telegram.dialogues.insert(
            key,
            Dialogue {
                title: peer_title(client, &input, key).await,
                history,
                history_loaded: true,
            },
        );
    }
    Ok(())
}

async fn seed_all_dialogues(client: &Client, telegram: &Telegram) -> Result<()> {
    let mut dialogs = client.iter_dialogs();
    let mut seen = HashSet::new();

    while let Some(dialog) = dialogs.next(client).await.map_err(|e| eyre!("{e}"))? {
        let Some(peer) = dialog.peer() else {
            continue;
        };
        let Some(key) = ChatKey::from_peer(peer) else {
            continue;
        };
        if !seen.insert(key) {
            continue;
        }

        let history = dialog
            .message
            .as_ref()
            .and_then(message_from_raw)
            .into_iter()
            .collect();
        telegram.dialogues.insert(
            key,
            Dialogue {
                title: dialog.title(),
                history,
                history_loaded: false,
            },
        );
    }
    Ok(())
}

fn message_from_raw(message: &tl::enums::Message) -> Option<Message> {
    let tl::enums::Message::Message(message) = message else {
        return None;
    };
    let text = message.message.trim();
    if text.is_empty() {
        return None;
    }
    Some(Message {
        id: message.id,
        text: text.to_string(),
        outgoing: message.out,
        date: message.date,
    })
}

#[cfg(test)]
mod tests {
    use super::same_folder_name;

    #[test]
    fn same_folder_name_supports_cyrillic() {
        assert!(same_folder_name("Тест", "тест"));
    }
}
