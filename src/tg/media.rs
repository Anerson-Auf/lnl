use ferogram::media::{Document, Downloadable, Photo, Sticker};
use ferogram::tl;
use ferogram::update::IncomingMessage;

use crate::config::types::{MediaInfo, MediaKind, Message, StickerFormat};

pub fn message_from_incoming(message: &IncomingMessage) -> Option<Message> {
    let text = message.text().unwrap_or("").to_string();
    let media = message.media().and_then(media_info);
    if text.trim().is_empty() && media.is_none() {
        return None;
    }

    Some(Message {
        id: message.id(),
        text,
        outgoing: message.outgoing(),
        date: message.date(),
        media,
    })
}

pub fn message_from_raw(message: &tl::enums::Message) -> Option<Message> {
    let tl::enums::Message::Message(message) = message else {
        return None;
    };
    let media = message.media.as_ref().and_then(media_info);
    if message.message.trim().is_empty() && media.is_none() {
        return None;
    }

    Some(Message {
        id: message.id,
        text: message.message.clone(),
        outgoing: message.out,
        date: message.date,
        media,
    })
}

pub fn media_info(media: &tl::enums::MessageMedia) -> Option<MediaInfo> {
    if let Some(sticker) = Sticker::from_media(media) {
        return Some(sticker_info(sticker, media));
    }
    if let Some(photo) = direct_photo(media) {
        return Some(photo_info(photo, media));
    }
    direct_document(media).map(|document| document_info(document, media))
}

pub fn media_label(media: &MediaInfo) -> &'static str {
    match media.kind {
        MediaKind::Sticker => "Стикер",
        MediaKind::Photo => "Фото",
        MediaKind::File => "Файл",
        MediaKind::Audio => "Аудио",
        MediaKind::Video => "Видео",
        MediaKind::Voice => "Голосовое сообщение",
        MediaKind::VideoNote => "Видеосообщение",
    }
}

fn direct_photo(media: &tl::enums::MessageMedia) -> Option<Photo> {
    let tl::enums::MessageMedia::Photo(media) = media else {
        return None;
    };
    let Some(tl::enums::Photo::Photo(photo)) = &media.photo else {
        return None;
    };
    Some(Photo::from_raw(photo.clone()))
}

fn direct_document(media: &tl::enums::MessageMedia) -> Option<Document> {
    let tl::enums::MessageMedia::Document(media) = media else {
        return None;
    };
    let Some(tl::enums::Document::Document(document)) = &media.document else {
        return None;
    };
    Some(Document::from_raw(document.clone()))
}

fn media_flags(media: &tl::enums::MessageMedia) -> (bool, bool, bool, bool) {
    match media {
        tl::enums::MessageMedia::Photo(value) => (value.spoiler, false, false, false),
        tl::enums::MessageMedia::Document(value) => {
            (value.spoiler, value.voice, value.round, value.video)
        }
        _ => (false, false, false, false),
    }
}

fn photo_info(photo: Photo, media: &tl::enums::MessageMedia) -> MediaInfo {
    let (spoiler, _, _, _) = media_flags(media);
    MediaInfo {
        kind: MediaKind::Photo,
        mime_type: Some("image/jpeg".to_string()),
        size: photo.size().map(|size| size as u64),
        file_name: Some(format!("photo-{}.jpg", photo.id())),
        duration_seconds: None,
        width: None,
        height: None,
        emoji: None,
        sticker_format: None,
        downloadable: true,
        spoiler,
    }
}

fn sticker_info(sticker: Sticker, media: &tl::enums::MessageMedia) -> MediaInfo {
    let mime_type = sticker.mime_type().to_string();
    let sticker_format = if mime_type == "application/x-tgsticker" {
        StickerFormat::Animated
    } else if sticker.is_video() || mime_type == "video/webm" {
        StickerFormat::Video
    } else {
        StickerFormat::Static
    };
    let (spoiler, _, _, _) = media_flags(media);
    MediaInfo {
        kind: MediaKind::Sticker,
        mime_type: Some(mime_type),
        size: sticker.size().map(|size| size as u64),
        file_name: sticker.inner.file_name().map(safe_received_filename),
        duration_seconds: None,
        width: None,
        height: None,
        emoji: sticker.emoji().map(str::to_string),
        sticker_format: Some(sticker_format),
        downloadable: true,
        spoiler,
    }
}

fn document_info(document: Document, media: &tl::enums::MessageMedia) -> MediaInfo {
    let (spoiler, outer_voice, outer_round, outer_video) = media_flags(media);
    let mut kind = if outer_voice {
        MediaKind::Voice
    } else if outer_round {
        MediaKind::VideoNote
    } else if outer_video {
        MediaKind::Video
    } else {
        MediaKind::File
    };
    let mut duration_seconds = None;
    let mut width = None;
    let mut height = None;

    for attribute in &document.raw.attributes {
        match attribute {
            tl::enums::DocumentAttribute::Audio(audio) => {
                kind = if audio.voice {
                    MediaKind::Voice
                } else {
                    MediaKind::Audio
                };
                duration_seconds = Some(audio.duration.max(0) as u32);
            }
            tl::enums::DocumentAttribute::Video(video) => {
                kind = if video.round_message {
                    MediaKind::VideoNote
                } else {
                    MediaKind::Video
                };
                duration_seconds = Some(video.duration.max(0.0).round() as u32);
                width = Some(video.w);
                height = Some(video.h);
            }
            _ => {}
        }
    }

    let mime_type = document.mime_type().to_string();
    MediaInfo {
        kind,
        mime_type: Some(mime_type),
        size: Some(document.size().max(0) as u64),
        file_name: document.file_name().map(safe_received_filename),
        duration_seconds,
        width,
        height,
        emoji: None,
        sticker_format: None,
        downloadable: true,
        spoiler,
    }
}

fn safe_received_filename(raw: &str) -> String {
    let normalized = raw.replace('\\', "/");
    let base = normalized.rsplit('/').next().unwrap_or("");
    let filtered = base
        .chars()
        .filter(|character| {
            !character.is_control()
                && !matches!(*character, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
        .collect::<String>();
    let trimmed = filtered.trim().trim_matches('.').trim();
    if trimmed.is_empty() {
        return "telegram-file".to_string();
    }
    let mut end = trimmed.len().min(120);
    while !trimmed.is_char_boundary(end) {
        end -= 1;
    }
    trimmed[..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::{media_label, message_from_raw, safe_received_filename};
    use crate::config::types::{MediaInfo, MediaKind};

    #[test]
    fn media_labels_cover_supported_kinds() {
        let info = MediaInfo {
            kind: MediaKind::VideoNote,
            mime_type: None,
            size: None,
            file_name: None,
            duration_seconds: None,
            width: None,
            height: None,
            emoji: None,
            sticker_format: None,
            downloadable: true,
            spoiler: false,
        };
        assert_eq!(media_label(&info), "Видеосообщение");
    }

    #[test]
    fn service_messages_are_not_relay_messages() {
        let raw = ferogram::tl::enums::Message::Empty(ferogram::tl::types::MessageEmpty {
            id: 1,
            peer_id: None,
        });
        assert!(message_from_raw(&raw).is_none());
    }

    #[test]
    fn received_filenames_drop_paths_controls_and_bidi_marks() {
        assert_eq!(
            safe_received_filename("../folder/report\u{202e}cod.exe"),
            "reportcod.exe"
        );
        assert_eq!(safe_received_filename("..\r\n"), "telegram-file");
        assert!(safe_received_filename(&"я".repeat(100)).len() <= 120);
    }
}
