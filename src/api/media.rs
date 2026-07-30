use std::io;
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{DefaultBodyLimit, Multipart, Path, State};
use axum::http::header::{
    ACCEPT_RANGES, CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE,
    RANGE,
};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::middleware;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use ferogram::media::size_from_media;
use ferogram::tl;
use ferogram::{Client, InputMessage, UploadedFile};
use futures_util::stream;
use serde::Serialize;
use tokio::io::AsyncWriteExt;

use crate::config::types::{ChatKey, MediaInfo, MediaKind, Message};
use crate::tg::media::{media_info, message_from_incoming};

use super::admin::{AdminAccess, require_admin};
use super::state::{AppState, SessionState};

const MAX_UPLOAD_BYTES: usize = 50 * 1024 * 1024;
const MAX_DOWNLOAD_BYTES: usize = 100 * 1024 * 1024;
const MULTIPART_OVERHEAD: usize = 64 * 1024;
const MAX_CAPTION_CHARS: usize = 1024;
const MAX_CAPTION_BYTES: usize = 4096;
const MAX_MEDIA_SECONDS: u32 = 60;
const DOWNLOAD_CHUNK_BYTES: i32 = 512 * 1024;
const MAX_MP4_SAMPLES: u32 = 18_000;
const MAX_MP4_SAMPLE_BYTES: u32 = 16 * 1024 * 1024;

pub fn router(state: AppState<Client>, access: AdminAccess) -> Router {
    let upload =
        post(send_media).layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES + MULTIPART_OVERHEAD));
    Router::new()
        .route(
            "/api/admin/sessions/{session_id}/messages/{peer_id}/media",
            upload,
        )
        .route(
            "/api/admin/sessions/{session_id}/messages/{peer_id}/{message_id}/media",
            get(download_media),
        )
        .route(
            "/api/admin/sessions/{session_id}/chats/{peer_id}/pin",
            put(pin_chat).delete(unpin_chat),
        )
        .route_layer(middleware::from_fn_with_state(access, require_admin))
        .with_state(state)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UploadKind {
    Photo,
    File,
    Voice,
    VideoNote,
}

impl UploadKind {
    fn parse(value: &str) -> Result<Self, MediaError> {
        match value {
            "photo" => Ok(Self::Photo),
            "file" => Ok(Self::File),
            "voice" => Ok(Self::Voice),
            "video_note" => Ok(Self::VideoNote),
            _ => Err(MediaError::bad_input(
                "invalid_kind",
                "kind должен быть photo, file, voice или video_note",
            )),
        }
    }
}

struct UploadRequest {
    kind: UploadKind,
    caption: String,
    duration_seconds: u32,
    side: i32,
    file_name: String,
    sniffed_mime: &'static str,
    size: usize,
    temp: TempUpload,
}

struct TempUpload {
    directory: PathBuf,
    path: PathBuf,
}

impl Drop for TempUpload {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_dir(&self.directory);
    }
}

#[derive(Serialize)]
struct MediaErrorBody {
    error: &'static str,
    code: &'static str,
}

#[derive(Debug)]
struct MediaError {
    status: StatusCode,
    code: &'static str,
    message: &'static str,
    range_total: Option<usize>,
}

impl MediaError {
    fn bad_input(code: &'static str, message: &'static str) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code,
            message,
            range_total: None,
        }
    }

    fn telegram(message: &'static str) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            code: "telegram_error",
            message,
            range_total: None,
        }
    }

    fn response(self) -> Response {
        let mut response = (
            self.status,
            [(CACHE_CONTROL, "no-store")],
            Json(MediaErrorBody {
                error: self.message,
                code: self.code,
            }),
        )
            .into_response();
        if let Some(total) = self.range_total
            && let Ok(value) = HeaderValue::from_str(&format!("bytes */{total}"))
        {
            response.headers_mut().insert(CONTENT_RANGE, value);
        }
        response
    }
}

#[derive(Serialize)]
struct SendMediaResponse {
    ok: bool,
    message: Message,
}

async fn send_media(
    State(state): State<AppState<Client>>,
    Path((session_id, peer_id)): Path<(String, i64)>,
    multipart: Multipart,
) -> Response {
    match send_media_inner(&state, &session_id, peer_id, multipart).await {
        Ok(message) => Json(SendMediaResponse { ok: true, message }).into_response(),
        Err(error) => error.response(),
    }
}

async fn send_media_inner(
    state: &AppState<Client>,
    session_id: &str,
    peer_id: i64,
    multipart: Multipart,
) -> Result<Message, MediaError> {
    let session = resolve_session(state, session_id)?;
    let key = existing_chat(&session, peer_id)?;
    let _upload = session.lock_upload().await;
    let mut request = parse_upload(multipart).await?;
    validate_upload(&mut request).await?;

    let uploaded = session
        .client
        .upload_sequential(&request.temp.path, None)
        .await
        .map_err(|_| MediaError::telegram("Не удалось загрузить файл в Telegram"))?;
    let media = match request.kind {
        UploadKind::Photo => uploaded.as_photo_media(),
        UploadKind::File => uploaded.as_document_media(),
        UploadKind::Voice => voice_media(&uploaded, request.duration_seconds),
        UploadKind::VideoNote => {
            video_note_media(&uploaded, request.duration_seconds, request.side)
        }
    };
    let sent = session
        .client
        .send_file(peer_id, media, &InputMessage::text(request.caption))
        .await
        .map_err(|_| MediaError::telegram("Telegram не принял вложение"))?;
    let message = message_from_incoming(&sent)
        .ok_or_else(|| MediaError::telegram("Telegram вернул пустое сообщение"))?;
    session.record_message(key, message.clone());
    Ok(message)
}

async fn parse_upload(mut multipart: Multipart) -> Result<UploadRequest, MediaError> {
    let mut kind = None;
    let mut caption = None;
    let mut file = None;

    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|_| MediaError::bad_input("invalid_multipart", "Некорректная форма загрузки"))?
    {
        let name = field
            .name()
            .ok_or_else(|| MediaError::bad_input("unnamed_field", "Поле формы без имени"))?
            .to_string();
        match name.as_str() {
            "kind" => {
                reject_duplicate(kind.is_some(), "duplicate_kind")?;
                let value =
                    read_text_field(&mut field, 32, "invalid_kind", "Некорректный kind").await?;
                kind = Some(UploadKind::parse(value.trim())?);
            }
            "caption" => {
                reject_duplicate(caption.is_some(), "duplicate_caption")?;
                caption = Some(
                    read_text_field(
                        &mut field,
                        MAX_CAPTION_BYTES,
                        "invalid_caption",
                        "Некорректная подпись",
                    )
                    .await?,
                );
            }
            "file" => {
                reject_duplicate(file.is_some(), "duplicate_file")?;
                let file_name = sanitize_filename(field.file_name().unwrap_or("file"))?;
                let (temp, mut output) = create_temp_upload(&file_name).await?;
                let mut size = 0usize;
                let mut header = Vec::with_capacity(64);
                while let Some(chunk) = field.chunk().await.map_err(|_| {
                    MediaError::bad_input("invalid_file", "Не удалось прочитать файл")
                })? {
                    size = size.saturating_add(chunk.len());
                    if size > MAX_UPLOAD_BYTES {
                        return Err(MediaError {
                            status: StatusCode::PAYLOAD_TOO_LARGE,
                            code: "file_too_large",
                            message: "Файл превышает 50 МиБ",
                            range_total: None,
                        });
                    }
                    if header.len() < 64 {
                        let take = (64 - header.len()).min(chunk.len());
                        header.extend_from_slice(&chunk[..take]);
                    }
                    output.write_all(&chunk).await.map_err(|_| MediaError {
                        status: StatusCode::INTERNAL_SERVER_ERROR,
                        code: "temp_write_failed",
                        message: "Не удалось сохранить временный файл",
                        range_total: None,
                    })?;
                }
                output.flush().await.map_err(|_| MediaError {
                    status: StatusCode::INTERNAL_SERVER_ERROR,
                    code: "temp_write_failed",
                    message: "Не удалось сохранить временный файл",
                    range_total: None,
                })?;
                if size == 0 {
                    return Err(MediaError::bad_input("empty_file", "Файл пуст"));
                }
                file = Some((
                    temp,
                    file_name,
                    sniff_mime(&header, size).unwrap_or("application/octet-stream"),
                    size,
                ));
            }
            _ => {
                return Err(MediaError::bad_input(
                    "unknown_field",
                    "Форма содержит неизвестное поле",
                ));
            }
        }
    }

    let kind =
        kind.ok_or_else(|| MediaError::bad_input("missing_kind", "Не указан kind вложения"))?;
    let (temp, file_name, sniffed_mime, size) =
        file.ok_or_else(|| MediaError::bad_input("missing_file", "Файл не выбран"))?;
    Ok(UploadRequest {
        kind,
        caption: caption.unwrap_or_default(),
        duration_seconds: 0,
        side: 0,
        file_name,
        sniffed_mime,
        size,
        temp,
    })
}

async fn read_text_field(
    field: &mut axum::extract::multipart::Field<'_>,
    limit: usize,
    code: &'static str,
    message: &'static str,
) -> Result<String, MediaError> {
    let mut bytes = Vec::new();
    while let Some(chunk) = field
        .chunk()
        .await
        .map_err(|_| MediaError::bad_input(code, message))?
    {
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err(MediaError::bad_input(code, message));
        }
        bytes.extend_from_slice(&chunk);
    }
    String::from_utf8(bytes).map_err(|_| MediaError::bad_input(code, message))
}

fn reject_duplicate(duplicate: bool, code: &'static str) -> Result<(), MediaError> {
    if duplicate {
        return Err(MediaError::bad_input(
            code,
            "Поле формы передано несколько раз",
        ));
    }
    Ok(())
}

async fn validate_upload(request: &mut UploadRequest) -> Result<(), MediaError> {
    if request.caption.chars().count() > MAX_CAPTION_CHARS
        || request.caption.len() > MAX_CAPTION_BYTES
    {
        return Err(MediaError::bad_input(
            "caption_too_long",
            "Подпись превышает лимит Telegram",
        ));
    }
    if request.size == 0 || request.size > MAX_UPLOAD_BYTES {
        return Err(MediaError::bad_input(
            "invalid_file_size",
            "Некорректный размер файла",
        ));
    }
    if request.file_name.is_empty() {
        return Err(MediaError::bad_input(
            "invalid_filename",
            "Некорректное имя файла",
        ));
    }

    match request.kind {
        UploadKind::Photo
            if !matches!(
                request.sniffed_mime,
                "image/jpeg" | "image/png" | "image/webp"
            ) =>
        {
            Err(MediaError::bad_input(
                "invalid_photo",
                "Фото должно быть JPEG, PNG или WebP",
            ))
        }
        UploadKind::Voice if request.sniffed_mime != "audio/ogg" => {
            return Err(MediaError::bad_input(
                "invalid_voice",
                "Голосовое сообщение должно быть OGG/Opus",
            ));
        }
        UploadKind::VideoNote if request.sniffed_mime != "video/mp4" => Err(MediaError::bad_input(
            "invalid_video_note",
            "Видеокружок должен быть MP4",
        )),
        _ => Ok(()),
    }?;

    match request.kind {
        UploadKind::Voice => {
            request.duration_seconds = probe_ogg_opus(request.temp.path.clone()).await?;
        }
        UploadKind::VideoNote => {
            let probe = probe_mp4_h264(request.temp.path.clone()).await?;
            request.duration_seconds = probe.duration_seconds;
            request.side = probe.side;
        }
        UploadKind::Photo | UploadKind::File => {}
    }
    Ok(())
}

async fn probe_ogg_opus(path: PathBuf) -> Result<u32, MediaError> {
    tokio::task::spawn_blocking(move || probe_ogg_opus_sync(&path))
        .await
        .map_err(|_| invalid_voice())?
}

fn probe_ogg_opus_sync(path: &FsPath) -> Result<u32, MediaError> {
    use std::io::{Read, Seek, SeekFrom};

    let mut file = std::fs::File::open(path).map_err(|_| invalid_voice())?;
    let file_length = file.metadata().map_err(|_| invalid_voice())?.len();
    let mut last_granule = None;
    let mut pre_skip = None;
    let mut page_count = 0usize;
    let mut stream_serial = None;
    let mut expected_sequence = 0u32;
    let mut seen_eos = false;

    loop {
        let mut header = [0u8; 27];
        match file.read(&mut header[..1]) {
            Ok(0) if page_count > 0 => break,
            Ok(1) => {}
            _ => return Err(invalid_voice()),
        }
        file.read_exact(&mut header[1..])
            .map_err(|_| invalid_voice())?;
        if &header[..4] != b"OggS" || header[4] != 0 {
            return Err(invalid_voice());
        }
        if seen_eos {
            return Err(invalid_voice());
        }
        page_count += 1;
        if page_count > 100_000 {
            return Err(invalid_voice());
        }
        let flags = header[5];
        let serial = u32::from_le_bytes(header[14..18].try_into().expect("fixed Ogg header"));
        let sequence = u32::from_le_bytes(header[18..22].try_into().expect("fixed Ogg header"));
        if page_count == 1 {
            if flags & 0x02 == 0 || flags & 0x01 != 0 || sequence != 0 {
                return Err(invalid_voice());
            }
            stream_serial = Some(serial);
        } else if stream_serial != Some(serial)
            || flags & 0x02 != 0
            || sequence != expected_sequence
        {
            return Err(invalid_voice());
        }
        expected_sequence = sequence.wrapping_add(1);

        let granule = u64::from_le_bytes(header[6..14].try_into().expect("fixed Ogg header"));
        if granule != u64::MAX {
            if last_granule.is_some_and(|previous| granule < previous) {
                return Err(invalid_voice());
            }
            last_granule = Some(granule);
        }
        let segment_count = header[26] as usize;
        let mut lacing = vec![0u8; segment_count];
        file.read_exact(&mut lacing).map_err(|_| invalid_voice())?;
        let payload_len = lacing
            .iter()
            .try_fold(0usize, |total, value| total.checked_add(*value as usize))
            .ok_or_else(invalid_voice)?;
        let payload_start = file.stream_position().map_err(|_| invalid_voice())?;
        if payload_start
            .checked_add(payload_len as u64)
            .is_none_or(|end| end > file_length)
        {
            return Err(invalid_voice());
        }

        if pre_skip.is_none() {
            let mut prefix = vec![0u8; payload_len.min(64)];
            file.read_exact(&mut prefix).map_err(|_| invalid_voice())?;
            if !prefix.starts_with(b"OpusHead") || prefix.len() < 12 || prefix[8] == 0 {
                return Err(invalid_voice());
            }
            pre_skip = Some(u16::from_le_bytes([prefix[10], prefix[11]]) as u64);
            file.seek(SeekFrom::Current(
                (payload_len.saturating_sub(prefix.len())) as i64,
            ))
            .map_err(|_| invalid_voice())?;
        } else {
            file.seek(SeekFrom::Current(payload_len as i64))
                .map_err(|_| invalid_voice())?;
        }
        seen_eos = flags & 0x04 != 0;
    }

    if !seen_eos {
        return Err(invalid_voice());
    }
    let samples = last_granule
        .ok_or_else(invalid_voice)?
        .saturating_sub(pre_skip.ok_or_else(invalid_voice)?);
    bounded_duration(samples as f64 / 48_000.0, invalid_voice)
}

struct VideoNoteProbe {
    duration_seconds: u32,
    side: i32,
}

async fn probe_mp4_h264(path: PathBuf) -> Result<VideoNoteProbe, MediaError> {
    tokio::task::spawn_blocking(move || probe_mp4_h264_sync(&path))
        .await
        .map_err(|_| invalid_video_note())?
}

fn probe_mp4_h264_sync(path: &FsPath) -> Result<VideoNoteProbe, MediaError> {
    let file = std::fs::File::open(path).map_err(|_| invalid_video_note())?;
    let length = file.metadata().map_err(|_| invalid_video_note())?.len();
    let mut reader = mp4::Mp4Reader::read_header(std::io::BufReader::new(file), length)
        .map_err(|_| invalid_video_note())?;
    if reader.is_fragmented() {
        return Err(invalid_video_note());
    }

    let track = reader
        .tracks()
        .values()
        .find_map(|track| {
            if track.track_type().ok()? != mp4::TrackType::Video
                || track.media_type().ok()? != mp4::MediaType::H264
                || track.timescale() == 0
                || validate_mp4_sample_table(track, length).is_err()
            {
                return None;
            }
            let avc = track.trak.mdia.minf.stbl.stsd.avc1.as_ref()?;
            let sps = avc.avcc.sequence_parameter_sets.first()?.bytes.as_slice();
            let pps = avc.avcc.picture_parameter_sets.first()?.bytes.as_slice();
            if sps.first().map(|byte| byte & 0x1f) != Some(7)
                || pps.first().map(|byte| byte & 0x1f) != Some(8)
            {
                return None;
            }
            Some((
                track.track_id(),
                track.sample_count(),
                track.trak.mdia.mdhd.duration as f64 / track.timescale() as f64,
                avc.width as i32,
                avc.height as i32,
                (avc.avcc.length_size_minus_one & 0x03) as usize + 1,
            ))
        })
        .ok_or_else(|| {
            MediaError::bad_input(
                "invalid_video_note_codec",
                "Видеокружок должен содержать H.264 video",
            )
        })?;
    let (track_id, sample_count, duration, width, height, nal_length_size) = track;
    let first = reader
        .read_sample(track_id, 1)
        .map_err(|_| invalid_video_note())?
        .ok_or_else(invalid_video_note)?;
    if !contains_h264_slice(&first.bytes, nal_length_size) {
        return Err(invalid_video_note());
    }
    if sample_count > 1 {
        let last = reader
            .read_sample(track_id, sample_count)
            .map_err(|_| invalid_video_note())?
            .ok_or_else(invalid_video_note)?;
        if !contains_h264_slice(&last.bytes, nal_length_size) {
            return Err(invalid_video_note());
        }
    }

    let duration_seconds = bounded_duration(duration, invalid_video_note)?;
    let side = width.min(height);
    if !(64..=4096).contains(&side) {
        return Err(invalid_video_note());
    }
    Ok(VideoNoteProbe {
        duration_seconds,
        side,
    })
}

fn validate_mp4_sample_table(track: &mp4::Mp4Track, file_length: u64) -> Result<(), MediaError> {
    let stbl = &track.trak.mdia.minf.stbl;
    let stsz = &stbl.stsz;
    let sample_count = stsz.sample_count;
    if sample_count == 0 || sample_count > MAX_MP4_SAMPLES {
        return Err(invalid_video_note());
    }

    let total_sample_bytes = if stsz.sample_size == 0 {
        if stsz.sample_sizes.len() != sample_count as usize
            || stsz
                .sample_sizes
                .iter()
                .any(|size| *size == 0 || *size > MAX_MP4_SAMPLE_BYTES)
        {
            return Err(invalid_video_note());
        }
        stsz.sample_sizes
            .iter()
            .try_fold(0u64, |total, size| total.checked_add(u64::from(*size)))
            .ok_or_else(invalid_video_note)?
    } else {
        if stsz.sample_size > MAX_MP4_SAMPLE_BYTES {
            return Err(invalid_video_note());
        }
        u64::from(stsz.sample_size)
            .checked_mul(u64::from(sample_count))
            .ok_or_else(invalid_video_note)?
    };
    if total_sample_bytes == 0 || total_sample_bytes > file_length {
        return Err(invalid_video_note());
    }

    let stsc = &stbl.stsc;
    if stsc.entries.is_empty()
        || stsc.entries.iter().enumerate().any(|(index, entry)| {
            entry.first_chunk == 0
                || entry.samples_per_chunk == 0
                || entry.samples_per_chunk > sample_count
                || entry.sample_description_index == 0
                || entry.first_sample == 0
                || entry.first_sample > sample_count
                || index > 0
                    && (entry.first_chunk <= stsc.entries[index - 1].first_chunk
                        || entry.first_sample <= stsc.entries[index - 1].first_sample)
        })
    {
        return Err(invalid_video_note());
    }

    let timed_samples = stbl
        .stts
        .entries
        .iter()
        .try_fold(0u32, |total, entry| {
            if entry.sample_count == 0 || entry.sample_delta == 0 {
                None
            } else {
                total.checked_add(entry.sample_count)
            }
        })
        .ok_or_else(invalid_video_note)?;
    if timed_samples != sample_count {
        return Err(invalid_video_note());
    }
    Ok(())
}

fn contains_h264_slice(bytes: &[u8], length_size: usize) -> bool {
    if !(1..=4).contains(&length_size) {
        return false;
    }
    let mut offset = 0usize;
    let mut found_slice = false;
    while offset < bytes.len() {
        if offset
            .checked_add(length_size)
            .is_none_or(|end| end > bytes.len())
        {
            return false;
        }
        let mut length = 0usize;
        for byte in &bytes[offset..offset + length_size] {
            length = match length
                .checked_mul(256)
                .and_then(|value| value.checked_add(*byte as usize))
            {
                Some(length) => length,
                None => return false,
            };
        }
        offset += length_size;
        if length == 0
            || offset
                .checked_add(length)
                .is_none_or(|end| end > bytes.len())
        {
            return false;
        }
        found_slice |= matches!(bytes[offset] & 0x1f, 1 | 5);
        offset += length;
    }
    found_slice
}

fn bounded_duration(duration: f64, invalid: fn() -> MediaError) -> Result<u32, MediaError> {
    if !duration.is_finite() || duration <= 0.0 || duration > MAX_MEDIA_SECONDS as f64 {
        return Err(MediaError::bad_input(
            "media_duration_invalid",
            "Голосовое или видеокружок должны длиться от 1 до 60 секунд",
        ));
    }
    let rounded = duration.ceil() as u32;
    if rounded == 0 || rounded > MAX_MEDIA_SECONDS {
        return Err(invalid());
    }
    Ok(rounded)
}

fn invalid_voice() -> MediaError {
    MediaError::bad_input(
        "invalid_voice",
        "Голосовое сообщение должно быть корректным OGG/Opus до 60 секунд",
    )
}

fn invalid_video_note() -> MediaError {
    MediaError::bad_input(
        "invalid_video_note",
        "Видеокружок должен быть корректным MP4/H.264 до 60 секунд",
    )
}

fn voice_media(uploaded: &UploadedFile, duration_seconds: u32) -> tl::enums::InputMedia {
    let mut media = uploaded.as_document_media();
    let tl::enums::InputMedia::UploadedDocument(document) = &mut media else {
        unreachable!("as_document_media returned a non-document")
    };
    document.force_file = false;
    document.attributes.retain(|attribute| {
        !matches!(
            attribute,
            tl::enums::DocumentAttribute::Audio(_) | tl::enums::DocumentAttribute::Video(_)
        )
    });
    document
        .attributes
        .push(tl::enums::DocumentAttribute::Audio(
            tl::types::DocumentAttributeAudio {
                voice: true,
                duration: duration_seconds.min(MAX_MEDIA_SECONDS) as i32,
                title: None,
                performer: None,
                waveform: None,
            },
        ));
    media
}

fn video_note_media(
    uploaded: &UploadedFile,
    duration_seconds: u32,
    side: i32,
) -> tl::enums::InputMedia {
    let mut media = uploaded.as_document_media();
    let tl::enums::InputMedia::UploadedDocument(document) = &mut media else {
        unreachable!("as_document_media returned a non-document")
    };
    document.force_file = false;
    document.nosound_video = false;
    document.attributes.retain(|attribute| {
        !matches!(
            attribute,
            tl::enums::DocumentAttribute::Audio(_) | tl::enums::DocumentAttribute::Video(_)
        )
    });
    document
        .attributes
        .push(tl::enums::DocumentAttribute::Video(
            tl::types::DocumentAttributeVideo {
                round_message: true,
                supports_streaming: true,
                nosound: false,
                duration: duration_seconds.min(MAX_MEDIA_SECONDS) as f64,
                w: side,
                h: side,
                preload_prefix_size: None,
                video_start_ts: None,
                video_codec: None,
            },
        ));
    media
}

async fn download_media(
    State(state): State<AppState<Client>>,
    Path((session_id, peer_id, message_id)): Path<(String, i64, i32)>,
    headers: HeaderMap,
) -> Response {
    match download_media_inner(&state, &session_id, peer_id, message_id, &headers).await {
        Ok(response) => response,
        Err(error) => error.response(),
    }
}

async fn download_media_inner(
    state: &AppState<Client>,
    session_id: &str,
    peer_id: i64,
    message_id: i32,
    headers: &HeaderMap,
) -> Result<Response, MediaError> {
    if message_id <= 0 {
        return Err(MediaError::bad_input(
            "invalid_message_id",
            "Некорректный идентификатор сообщения",
        ));
    }
    let session = resolve_session(state, session_id)?;
    existing_chat(&session, peer_id)?;
    let permit = session.try_download_permit().ok_or(MediaError {
        status: StatusCode::TOO_MANY_REQUESTS,
        code: "download_busy",
        message: "Слишком много одновременных скачиваний для аккаунта",
        range_total: None,
    })?;
    let messages = session
        .client
        .get_messages(peer_id, &[message_id])
        .await
        .map_err(|_| MediaError::telegram("Не удалось получить вложение из Telegram"))?;
    let message = messages
        .into_iter()
        .find(|message| {
            message.id() == message_id
                && message
                    .peer_id()
                    .and_then(ChatKey::from_peer)
                    .map(ChatKey::bot_api_id)
                    == Some(peer_id)
        })
        .ok_or(MediaError {
            status: StatusCode::NOT_FOUND,
            code: "message_not_found",
            message: "Сообщение не найдено в выбранном чате",
            range_total: None,
        })?;
    let telegram_media = message.media().cloned().ok_or(MediaError {
        status: StatusCode::NOT_FOUND,
        code: "media_not_found",
        message: "В сообщении нет скачиваемого вложения",
        range_total: None,
    })?;
    let info = media_info(&telegram_media).ok_or(MediaError {
        status: StatusCode::UNSUPPORTED_MEDIA_TYPE,
        code: "unsupported_media",
        message: "Этот тип вложения пока не поддерживается",
        range_total: None,
    })?;
    let total = size_from_media(&telegram_media).ok_or(MediaError {
        status: StatusCode::UNSUPPORTED_MEDIA_TYPE,
        code: "unknown_media_size",
        message: "Telegram не сообщил размер вложения",
        range_total: None,
    })?;
    let requested = parse_range(headers.get(RANGE), total)?;
    let response_bytes = requested
        .map(|(start, end, _)| end - start + 1)
        .unwrap_or(total);
    if response_bytes > MAX_DOWNLOAD_BYTES {
        return Err(MediaError {
            status: StatusCode::PAYLOAD_TOO_LARGE,
            code: "download_too_large",
            message: "За один запрос можно скачать не более 100 МиБ",
            range_total: None,
        });
    }
    let mut iterator = session
        .client
        .iter_download(&telegram_media)
        .ok_or(MediaError {
            status: StatusCode::UNSUPPORTED_MEDIA_TYPE,
            code: "not_downloadable",
            message: "Telegram не позволяет скачать это вложение",
            range_total: None,
        })?
        .chunk_size(DOWNLOAD_CHUNK_BYTES);

    let (status, start, end, skip, remaining) = match requested {
        Some((start, end, _total)) => {
            let aligned = start / DOWNLOAD_CHUNK_BYTES as usize * DOWNLOAD_CHUNK_BYTES as usize;
            iterator = iterator.start_at(start as i64);
            (
                StatusCode::PARTIAL_CONTENT,
                start,
                Some(end),
                start - aligned,
                Some(end - start + 1),
            )
        }
        None => (StatusCode::OK, 0, None, 0, Some(total)),
    };
    let stream_state = DownloadState {
        iterator,
        skip,
        remaining,
        _permit: permit,
    };
    let body_stream = stream::try_unfold(stream_state, |mut state| async move {
        loop {
            if state.remaining == Some(0) {
                return Ok(None);
            }
            let Some(mut chunk) = state
                .iterator
                .next()
                .await
                .map_err(|error| io::Error::other(error.to_string()))?
            else {
                return Ok(None);
            };
            if state.skip >= chunk.len() {
                state.skip -= chunk.len();
                continue;
            }
            if state.skip > 0 {
                chunk.drain(..state.skip);
                state.skip = 0;
            }
            if let Some(remaining) = state.remaining {
                chunk.truncate(chunk.len().min(remaining));
                state.remaining = Some(remaining - chunk.len());
            }
            return Ok::<_, io::Error>(Some((Bytes::from(chunk), state)));
        }
    });

    let mut response = Response::new(Body::from_stream(body_stream));
    *response.status_mut() = status;
    let response_headers = response.headers_mut();
    response_headers.insert(CACHE_CONTROL, HeaderValue::from_static("private, no-store"));
    response_headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    response_headers.insert(ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    response_headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static(response_content_type(&info)),
    );
    response_headers.insert(
        CONTENT_DISPOSITION,
        HeaderValue::from_static(if info.kind == MediaKind::File {
            "attachment; filename=\"download\""
        } else {
            "inline"
        }),
    );
    if let Some(length) = remaining
        && let Ok(value) = HeaderValue::from_str(&length.to_string())
    {
        response_headers.insert(CONTENT_LENGTH, value);
    }
    if let Some(end) = end
        && let Ok(value) = HeaderValue::from_str(&format!("bytes {start}-{end}/{total}"))
    {
        response_headers.insert(CONTENT_RANGE, value);
    }
    Ok(response)
}

struct DownloadState {
    iterator: ferogram::media::DownloadIter,
    skip: usize,
    remaining: Option<usize>,
    _permit: tokio::sync::OwnedSemaphorePermit,
}

fn parse_range(
    range: Option<&HeaderValue>,
    total: usize,
) -> Result<Option<(usize, usize, usize)>, MediaError> {
    let Some(range) = range else {
        return Ok(None);
    };
    let value = range
        .to_str()
        .map_err(|_| MediaError::bad_input("invalid_range", "Некорректный Range"))?;
    let value = value
        .strip_prefix("bytes=")
        .filter(|value| !value.contains(','))
        .ok_or_else(|| MediaError::bad_input("invalid_range", "Поддерживается один byte range"))?;
    let (start, end) = value
        .split_once('-')
        .ok_or_else(|| MediaError::bad_input("invalid_range", "Некорректный Range"))?;
    if start.is_empty() {
        return Err(MediaError::bad_input(
            "invalid_range",
            "Suffix range не поддерживается",
        ));
    }
    let start = start
        .parse::<usize>()
        .map_err(|_| MediaError::bad_input("invalid_range", "Некорректный Range"))?;
    if start >= total {
        return Err(MediaError {
            status: StatusCode::RANGE_NOT_SATISFIABLE,
            code: "range_not_satisfiable",
            message: "Range выходит за размер вложения",
            range_total: Some(total),
        });
    }
    let end = if end.is_empty() {
        total - 1
    } else {
        end.parse::<usize>()
            .map_err(|_| MediaError::bad_input("invalid_range", "Некорректный Range"))?
            .min(total - 1)
    };
    if end < start {
        return Err(MediaError::bad_input(
            "invalid_range",
            "Конец Range меньше начала",
        ));
    }
    Ok(Some((start, end, total)))
}

fn response_content_type(info: &MediaInfo) -> &'static str {
    match info.kind {
        MediaKind::Photo => "image/jpeg",
        MediaKind::Sticker => match info.mime_type.as_deref() {
            Some("image/webp") => "image/webp",
            Some("video/webm") => "video/webm",
            _ => "application/octet-stream",
        },
        MediaKind::Voice | MediaKind::Audio => match info.mime_type.as_deref() {
            Some("audio/ogg") | Some("application/ogg") => "audio/ogg",
            Some("audio/webm") => "audio/webm",
            Some("audio/mpeg") => "audio/mpeg",
            Some("audio/mp4") => "audio/mp4",
            _ => "application/octet-stream",
        },
        MediaKind::Video | MediaKind::VideoNote => match info.mime_type.as_deref() {
            Some("video/mp4") => "video/mp4",
            Some("video/webm") => "video/webm",
            _ => "application/octet-stream",
        },
        MediaKind::File => "application/octet-stream",
    }
}

#[derive(Serialize)]
struct PinResponse {
    ok: bool,
    pinned: bool,
}

async fn pin_chat(
    State(state): State<AppState<Client>>,
    Path((session_id, peer_id)): Path<(String, i64)>,
) -> Response {
    set_chat_pinned(&state, &session_id, peer_id, true).await
}

async fn unpin_chat(
    State(state): State<AppState<Client>>,
    Path((session_id, peer_id)): Path<(String, i64)>,
) -> Response {
    set_chat_pinned(&state, &session_id, peer_id, false).await
}

async fn set_chat_pinned(
    state: &AppState<Client>,
    session_id: &str,
    peer_id: i64,
    pinned: bool,
) -> Response {
    let result = async {
        let session = resolve_session(state, session_id)?;
        let key = existing_chat(&session, peer_id)?;
        session
            .client
            .pin_dialog(peer_id, pinned)
            .await
            .map_err(|_| MediaError::telegram("Telegram не изменил закрепление"))?;
        session.record_chat_pinned(key, pinned);
        Ok::<_, MediaError>(())
    }
    .await;
    match result {
        Ok(()) => Json(PinResponse { ok: true, pinned }).into_response(),
        Err(error) => error.response(),
    }
}

fn resolve_session(
    state: &AppState<Client>,
    session_id: &str,
) -> Result<Arc<SessionState<Client>>, MediaError> {
    match state.session(session_id) {
        Some(session) => Ok(session),
        None if state.is_configured(session_id) => Err(MediaError {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code: "account_not_ready",
            message: "Аккаунт ещё не авторизован",
            range_total: None,
        }),
        None => Err(MediaError {
            status: StatusCode::NOT_FOUND,
            code: "session_not_found",
            message: "Telegram-сессия не найдена",
            range_total: None,
        }),
    }
}

fn existing_chat<C>(session: &SessionState<C>, peer_id: i64) -> Result<ChatKey, MediaError> {
    let key = ChatKey::from_bot_api_id(peer_id);
    if !session.telegram.dialogues.contains_key(&key) {
        return Err(MediaError {
            status: StatusCode::NOT_FOUND,
            code: "chat_not_found",
            message: "Чат не найден в выбранной сессии",
            range_total: None,
        });
    }
    Ok(key)
}

async fn create_temp_upload(file_name: &str) -> Result<(TempUpload, tokio::fs::File), MediaError> {
    for _ in 0..8 {
        let mut random = [0u8; 12];
        getrandom::fill(&mut random).map_err(|_| MediaError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "random_failed",
            message: "Не удалось подготовить загрузку",
            range_total: None,
        })?;
        let suffix = random
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let directory = std::env::temp_dir().join(format!("lnl-upload-{suffix}"));
        match tokio::fs::create_dir(&directory).await {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(_) => {
                return Err(MediaError {
                    status: StatusCode::INTERNAL_SERVER_ERROR,
                    code: "temp_create_failed",
                    message: "Не удалось подготовить временный файл",
                    range_total: None,
                });
            }
        }
        let path = directory.join(file_name);
        let temp = TempUpload { directory, path };
        set_private_permissions(&temp.directory, 0o700)?;
        let output = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp.path)
            .await
            .map_err(|_| MediaError {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                code: "temp_create_failed",
                message: "Не удалось подготовить временный файл",
                range_total: None,
            })?;
        set_private_permissions(&temp.path, 0o600)?;
        return Ok((temp, output));
    }
    Err(MediaError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        code: "temp_collision",
        message: "Не удалось подготовить временный файл",
        range_total: None,
    })
}

#[cfg(unix)]
fn set_private_permissions(path: &FsPath, mode: u32) -> Result<(), MediaError> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).map_err(|_| MediaError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        code: "temp_permissions_failed",
        message: "Не удалось защитить временный файл",
        range_total: None,
    })
}

#[cfg(not(unix))]
fn set_private_permissions(_path: &FsPath, _mode: u32) -> Result<(), MediaError> {
    Ok(())
}

fn sanitize_filename(raw: &str) -> Result<String, MediaError> {
    let normalized = raw.replace('\\', "/");
    let base = normalized.rsplit('/').next().unwrap_or("");
    let filtered = base
        .chars()
        .filter(|character| {
            !character.is_control()
                && !matches!(
                    *character,
                    '\u{202a}'
                        ..='\u{202e}'
                            | '\u{2066}'
                                ..='\u{2069}'
                )
        })
        .collect::<String>();
    let trimmed = filtered.trim().trim_matches('.').trim();
    if trimmed.is_empty() {
        return Err(MediaError::bad_input(
            "invalid_filename",
            "Некорректное имя файла",
        ));
    }
    let mut end = trimmed.len().min(120);
    while !trimmed.is_char_boundary(end) {
        end -= 1;
    }
    let name = trimmed[..end].to_string();
    if name == "." || name == ".." {
        return Err(MediaError::bad_input(
            "invalid_filename",
            "Некорректное имя файла",
        ));
    }
    Ok(name)
}

fn sniff_mime(header: &[u8], size: usize) -> Option<&'static str> {
    if header.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if header.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if header.len() >= 12 && &header[..4] == b"RIFF" && &header[8..12] == b"WEBP" {
        Some("image/webp")
    } else if header.starts_with(b"OggS")
        && header
            .windows(b"OpusHead".len())
            .any(|part| part == b"OpusHead")
    {
        Some("audio/ogg")
    } else if header.len() >= 12 && &header[4..8] == b"ftyp" {
        Some("video/mp4")
    } else if header.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
        Some("video/webm")
    } else if size > 0 {
        Some("application/octet-stream")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{
        parse_range, probe_mp4_h264_sync, probe_ogg_opus_sync, sanitize_filename, sniff_mime,
    };
    use axum::http::HeaderValue;

    static TEMP_ID: AtomicU64 = AtomicU64::new(0);

    fn with_temp_file<T>(bytes: &[u8], test: impl FnOnce(&Path) -> T) -> T {
        let id = TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("lnl-media-test-{}-{id}.bin", std::process::id()));
        std::fs::write(&path, bytes).unwrap();
        let result = test(&path);
        std::fs::remove_file(path).unwrap();
        result
    }

    fn ogg_page(serial: u32, sequence: u32, flags: u8, granule: u64, payload: &[u8]) -> Vec<u8> {
        assert!(payload.len() <= u8::MAX as usize);
        let mut page = vec![0u8; 27];
        page[..4].copy_from_slice(b"OggS");
        page[5] = flags;
        page[6..14].copy_from_slice(&granule.to_le_bytes());
        page[14..18].copy_from_slice(&serial.to_le_bytes());
        page[18..22].copy_from_slice(&sequence.to_le_bytes());
        page[26] = 1;
        page.push(payload.len() as u8);
        page.extend_from_slice(payload);
        page
    }

    fn ogg_opus(duration_samples: u64) -> Vec<u8> {
        ogg_opus_with_serial(duration_samples, 1)
    }

    fn ogg_opus_with_serial(duration_samples: u64, serial: u32) -> Vec<u8> {
        let pre_skip = 312u16;
        let mut opus_head = b"OpusHead".to_vec();
        opus_head.extend_from_slice(&[1, 1]);
        opus_head.extend_from_slice(&pre_skip.to_le_bytes());
        opus_head.extend_from_slice(&48_000u32.to_le_bytes());
        opus_head.extend_from_slice(&0i16.to_le_bytes());
        opus_head.push(0);
        let mut file = ogg_page(serial, 0, 0x02, 0, &opus_head);
        file.extend(ogg_page(
            serial,
            1,
            0x04,
            pre_skip as u64 + duration_samples,
            &[0],
        ));
        file
    }

    fn mp4_box(kind: &[u8; 4], payload: Vec<u8>) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(payload.len() + 8);
        bytes.extend_from_slice(&((payload.len() + 8) as u32).to_be_bytes());
        bytes.extend_from_slice(kind);
        bytes.extend(payload);
        bytes
    }

    fn synthetic_mp4_metadata(
        version: u8,
        timescale: u32,
        duration: u64,
        codec: &[u8; 4],
    ) -> Vec<u8> {
        let mut mvhd = vec![0u8; if version == 0 { 20 } else { 32 }];
        mvhd[0] = version;
        if version == 0 {
            mvhd[12..16].copy_from_slice(&timescale.to_be_bytes());
            mvhd[16..20].copy_from_slice(&(duration as u32).to_be_bytes());
        } else {
            mvhd[20..24].copy_from_slice(&timescale.to_be_bytes());
            mvhd[24..32].copy_from_slice(&duration.to_be_bytes());
        }

        let mut tkhd = vec![0u8; if version == 0 { 84 } else { 96 }];
        tkhd[0] = version;
        let (width, height) = if version == 0 { (76, 80) } else { (88, 92) };
        tkhd[width..width + 4].copy_from_slice(&(640u32 << 16).to_be_bytes());
        tkhd[height..height + 4].copy_from_slice(&(480u32 << 16).to_be_bytes());

        let mut stsd = vec![0u8; 8];
        stsd[4..8].copy_from_slice(&1u32.to_be_bytes());
        stsd.extend_from_slice(&8u32.to_be_bytes());
        stsd.extend_from_slice(codec);

        let stbl = mp4_box(b"stbl", mp4_box(b"stsd", stsd));
        let minf = mp4_box(b"minf", stbl);
        let mdia = mp4_box(b"mdia", minf);
        let mut trak_payload = mp4_box(b"tkhd", tkhd);
        trak_payload.extend(mdia);
        let trak = mp4_box(b"trak", trak_payload);
        let mut moov_payload = mp4_box(b"mvhd", mvhd);
        moov_payload.extend(trak);
        mp4_box(b"moov", moov_payload)
    }

    fn mp4_video_note(duration_ms: u32, slice_type: u8) -> Vec<u8> {
        let config = mp4::Mp4Config {
            major_brand: "isom".parse().unwrap(),
            minor_version: 512,
            compatible_brands: vec![
                "isom".parse().unwrap(),
                "iso2".parse().unwrap(),
                "avc1".parse().unwrap(),
                "mp41".parse().unwrap(),
            ],
            timescale: 1_000,
        };
        let cursor = std::io::Cursor::new(Vec::new());
        let mut writer = mp4::Mp4Writer::write_start(cursor, &config).unwrap();
        writer
            .add_track(
                &mp4::AvcConfig {
                    width: 640,
                    height: 480,
                    seq_param_set: vec![0x67, 0x42, 0x00, 0x1e, 0xe9, 0x01, 0x40],
                    pic_param_set: vec![0x68, 0xce, 0x06, 0xe2],
                }
                .into(),
            )
            .unwrap();
        let sample_bytes = vec![0, 0, 0, 2, slice_type, 0x88];
        writer
            .write_sample(
                1,
                &mp4::Mp4Sample {
                    start_time: 0,
                    duration: duration_ms,
                    rendering_offset: 0,
                    is_sync: true,
                    bytes: mp4::Bytes::from(sample_bytes),
                },
            )
            .unwrap();
        writer.write_end().unwrap();
        writer.into_writer().into_inner()
    }

    fn patch_stsz(bytes: &mut [u8], sample_size: u32, sample_count: u32) {
        let offset = bytes
            .windows(4)
            .position(|window| window == b"stsz")
            .expect("generated MP4 contains stsz");
        bytes[offset + 8..offset + 12].copy_from_slice(&sample_size.to_be_bytes());
        bytes[offset + 12..offset + 16].copy_from_slice(&sample_count.to_be_bytes());
    }

    #[test]
    fn filenames_drop_paths_controls_and_stay_bounded() {
        assert_eq!(sanitize_filename("../../hello.txt").unwrap(), "hello.txt");
        assert_eq!(
            sanitize_filename("..\\folder\\hello\r\n.txt").unwrap(),
            "hello.txt"
        );
        assert!(sanitize_filename("../..").is_err());
        assert_eq!(
            sanitize_filename("report\u{202e}cod.exe").unwrap(),
            "reportcod.exe"
        );
        assert!(sanitize_filename(&"я".repeat(100)).unwrap().len() <= 120);
    }

    #[test]
    fn magic_detection_does_not_trust_extensions() {
        assert_eq!(sniff_mime(b"\x89PNG\r\n\x1a\nrest", 12), Some("image/png"));
        assert_eq!(
            sniff_mime(b"OggS\x00\x00\x00\x00OpusHead", 16),
            Some("audio/ogg")
        );
        assert_eq!(sniff_mime(b"\0\0\0\x18ftypisom", 12), Some("video/mp4"));
        assert_eq!(
            sniff_mime(b"plain text", 10),
            Some("application/octet-stream")
        );
    }

    #[test]
    fn one_bounded_http_range_is_supported() {
        assert_eq!(
            parse_range(Some(&HeaderValue::from_static("bytes=10-19")), 100).unwrap(),
            Some((10, 19, 100))
        );
        assert_eq!(
            parse_range(Some(&HeaderValue::from_static("bytes=90-")), 100).unwrap(),
            Some((90, 99, 100))
        );
        assert!(parse_range(Some(&HeaderValue::from_static("bytes=0-1,3-4")), 100).is_err());
        assert!(parse_range(Some(&HeaderValue::from_static("bytes=-10")), 100).is_err());
        let error = parse_range(Some(&HeaderValue::from_static("bytes=100-")), 100).unwrap_err();
        assert_eq!(error.range_total, Some(100));
    }

    #[test]
    fn ogg_opus_duration_uses_granule_and_pre_skip() {
        let exact_limit = ogg_opus(60 * 48_000);
        assert_eq!(
            with_temp_file(&exact_limit, probe_ogg_opus_sync).unwrap(),
            60
        );

        let too_long = ogg_opus(60 * 48_000 + 1);
        assert!(with_temp_file(&too_long, probe_ogg_opus_sync).is_err());
    }

    #[test]
    fn ogg_opus_rejects_truncated_pages() {
        let mut truncated = ogg_opus(48_000);
        truncated.extend_from_slice(b"Ogg");
        assert!(with_temp_file(&truncated, probe_ogg_opus_sync).is_err());

        let mut truncated_payload = ogg_opus(48_000);
        truncated_payload.pop();
        assert!(with_temp_file(&truncated_payload, probe_ogg_opus_sync).is_err());
        assert!(with_temp_file(b"not ogg", probe_ogg_opus_sync).is_err());
    }

    #[test]
    fn ogg_opus_rejects_chained_or_unfinished_streams() {
        let mut chained = ogg_opus_with_serial(60 * 48_000, 1);
        chained.extend(ogg_opus_with_serial(60 * 48_000, 2));
        assert!(with_temp_file(&chained, probe_ogg_opus_sync).is_err());

        let mut unfinished = ogg_opus(48_000);
        unfinished[5] &= !0x04;
        let second_page = 27 + 1 + 19;
        unfinished[second_page + 5] &= !0x04;
        assert!(with_temp_file(&unfinished, probe_ogg_opus_sync).is_err());
    }

    #[test]
    fn mp4_probe_requires_real_h264_samples() {
        let valid = mp4_video_note(60_000, 0x65);
        let probe = with_temp_file(&valid, probe_mp4_h264_sync).unwrap();
        assert_eq!(probe.duration_seconds, 60);
        assert_eq!(probe.side, 480);

        let metadata_only = synthetic_mp4_metadata(0, 1_000, 1_000, b"avc1");
        assert!(with_temp_file(&metadata_only, probe_mp4_h264_sync).is_err());

        let non_slice = mp4_video_note(1_000, 0x06);
        assert!(with_temp_file(&non_slice, probe_mp4_h264_sync).is_err());
    }

    #[test]
    fn mp4_probe_rejects_unbounded_sample_tables_before_reading() {
        let mut huge_sample = mp4_video_note(1_000, 0x65);
        patch_stsz(&mut huge_sample, u32::MAX, 1);
        assert!(with_temp_file(&huge_sample, probe_mp4_h264_sync).is_err());

        let mut huge_count = mp4_video_note(1_000, 0x65);
        patch_stsz(&mut huge_count, 1, u32::MAX);
        assert!(with_temp_file(&huge_count, probe_mp4_h264_sync).is_err());
    }

    #[test]
    fn mp4_probe_rejects_bad_codec_duration_and_boxes() {
        let wrong_codec = synthetic_mp4_metadata(0, 1_000, 1_000, b"hev1");
        assert!(with_temp_file(&wrong_codec, probe_mp4_h264_sync).is_err());

        let too_long = mp4_video_note(60_001, 0x65);
        assert!(with_temp_file(&too_long, probe_mp4_h264_sync).is_err());

        let malformed = [0, 0, 0, 4, b'm', b'o', b'o', b'v'];
        assert!(with_temp_file(&malformed, probe_mp4_h264_sync).is_err());
    }
}
