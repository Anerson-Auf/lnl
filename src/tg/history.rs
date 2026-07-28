use color_eyre::{eyre::eyre, Result};
use ferogram::tl;
use ferogram::Client;
use std::collections::HashSet;

use crate::config::types::{ChatKey, Dialogue, Message, Telegram};

fn text_of(title: &tl::enums::TextWithEntities) -> &str {
    match title {
        tl::enums::TextWithEntities::TextWithEntities(t) => t.text.as_str(),
    }
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

pub async fn list_folders(client: &Client) -> Result<Vec<(i32, String)>> {
    let resp = client
        .invoke(&tl::functions::messages::GetDialogFilters {})
        .await
        .map_err(|e| eyre!("{e}"))?;

    let tl::enums::messages::DialogFilters::DialogFilters(df) = resp;
    let mut out = Vec::new();
    for f in df.filters {
        match f {
            tl::enums::DialogFilter::DialogFilter(f) => {
                out.push((f.id, text_of(&f.title).to_string()));
            }
            tl::enums::DialogFilter::Chatlist(f) => {
                out.push((f.id, text_of(&f.title).to_string()));
            }
            tl::enums::DialogFilter::Default => {
                out.push((0, "All chats".to_string()));
            }
        }
    }
    Ok(out)
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
    if let Ok(peer) = ferogram::PeerRef::from(key.bot_api_id())
        .resolve(client)
        .await
    {
        if let tl::enums::Peer::User(u) = &peer {
            if let Ok(users) = client.get_users_by_id(&[u.user_id]).await {
                if let Some(Some(user)) = users.into_iter().next() {
                    let first = user.first_name().unwrap_or("");
                    let last = user.last_name().unwrap_or("");
                    let name = format!("{first} {last}").trim().to_string();
                    if !name.is_empty() {
                        return name;
                    }
                    if let Some(uname) = user.username() {
                        return format!("@{uname}");
                    }
                }
            }
        }
    }
    format!("{}", key.bot_api_id())
}

pub async fn seed_dialogues_from_folder(
    client: &Client,
    telegram: &Telegram,
    folder_name: &str,
    per_chat: i32,
) -> Result<()> {
    let folder_name = folder_name.trim();
    if folder_name.is_empty() {
        return Err(eyre!("TG_FOLDER пуст — укажи имя папки Telegram"));
    }

    let resp = client
        .invoke(&tl::functions::messages::GetDialogFilters {})
        .await
        .map_err(|e| eyre!("{e}"))?;

    let tl::enums::messages::DialogFilters::DialogFilters(df) = resp;

    let wanted = folder_name.to_lowercase();
    let filter = df.filters.into_iter().find(|f| match f {
        tl::enums::DialogFilter::DialogFilter(f) => {
            text_of(&f.title).eq_ignore_ascii_case(&wanted)
        }
        tl::enums::DialogFilter::Chatlist(f) => text_of(&f.title).eq_ignore_ascii_case(&wanted),
        tl::enums::DialogFilter::Default => wanted == "all chats" || wanted == "all",
    });

    let Some(filter) = filter else {
        let names = list_folders(client)
            .await?
            .into_iter()
            .map(|(_, n)| format!("«{n}»"))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(eyre!("папка «{folder_name}» не найдена. Есть: {names}"));
    };

    let peers = peers_from_filter(&filter);
    if peers.is_empty() {
        return Err(eyre!(
            "папка «{folder_name}» пустая (нет include/pinned peers). \
             Добавь чаты вручную в папку."
        ));
    }

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

        let mut history: Vec<Message> = page
            .messages
            .into_iter()
            .filter_map(|m| {
                let text = m.text()?.trim();
                if text.is_empty() {
                    return None;
                }
                Some(Message {
                    id: m.id(),
                    text: text.to_string(),
                    outgoing: m.outgoing(),
                    date: m.date(),
                })
            })
            .collect();
        // Telegram отдаёт новые первыми — для UI старые сверху.
        history.reverse();

        let title = peer_title(client, &input, key).await;

        telegram.dialogues.insert(
            key,
            Dialogue {
                title,
                history,
            },
        );
    }

    Ok(())
}
