# LNL Relay API (для Android)

База: `https://<host>` через TLS reverse proxy; локальный debug может
использовать `http://<host>:8080`.
`peer_id` — Bot API id (`i64`).

## Сессии

### `GET /api/sessions`

Настроенные Telegram-сессии в порядке запуска:

```json
[
  { "id": "default", "is_default": true },
  { "id": "work", "is_default": false }
]
```

Ответ не содержит пути session-файлов, `api_hash` или настройки прокси.
До завершения входа слот не попадает в этот публичный список. Настроенный, но
ещё не готовый scoped account возвращает `503`; неизвестный id — `404`.

### Scoped API

Для явного выбора Telegram-аккаунта используй:

- `GET /api/sessions/{session_id}/chats`
- `GET /api/sessions/{session_id}/messages/{peer_id}`
- `POST /api/sessions/{session_id}/messages/{peer_id}`
- `WS /api/sessions/{session_id}/ws`

Неизвестный `session_id` возвращает `404`; fallback на другой аккаунт
запрещён. Один WebSocket получает события только выбранной сессии.

Старые `GET /api/chats`, `GET|POST /api/messages/{peer_id}` и `WS /ws`
сохранены без изменения формы данных. Они всегда обращаются к сессии с
`is_default: true`.

## REST

### `GET /api/chats`

Список чатов default-сессии. Scoped-вариант:
`GET /api/sessions/{session_id}/chats`.

```json
[
  {
    "peer_id": 123456789,
    "title": "Имя",
    "last_message": "текст или null"
  }
]
```

### `GET /api/messages/{peer_id}`

История default-сессии (старые → новые). Scoped-вариант:
`GET /api/sessions/{session_id}/messages/{peer_id}`.

```json
[
  {
    "id": 42,
    "text": "привет",
    "outgoing": false,
    "date": 1710000000
  }
]
```

`404` если чата нет в папке:

```json
{ "error": "нет чата …" }
```

У сообщения с Telegram-вложением появляется необязательное поле `media`.
Текстовые сообщения сохраняют прежнюю JSON-форму. Поддерживаются
`sticker`, `photo`, `file`, `audio`, `video`, `voice` и `video_note`:

```json
{
  "id": 45,
  "text": "",
  "outgoing": false,
  "date": 1710000003,
  "media": {
    "kind": "voice",
    "mime_type": "audio/ogg",
    "size": 18240,
    "duration_seconds": 7,
    "downloadable": true,
    "spoiler": false
  }
}
```

### `POST /api/messages/{peer_id}`

Отправка через default-сессию. Scoped-вариант:
`POST /api/sessions/{session_id}/messages/{peer_id}`.

Тело:

```json
{ "text": "ответ" }
```

Ответ:

```json
{
  "ok": true,
  "message": {
    "id": 43,
    "text": "ответ",
    "outgoing": true,
    "date": 1710000001
  }
}
```

## WebSocket `GET /ws`

Подписка на default-сессию. Для явного аккаунта:
`GET /api/sessions/{session_id}/ws`. Сервер пушит JSON:

```json
{
  "type": "new_message",
  "peer_id": 123456789,
  "message": {
    "id": 44,
    "text": "новое",
    "outgoing": false,
    "date": 1710000002
  }
}
```

Клиент может только читать; отправка — через REST.
Изменение закрепа в основном списке или архиве Telegram приходит отдельным событием:

```json
{ "type": "chat_pinned", "peer_id": 123456789, "pinned": true }
```

## Рекомендации Android

1. Сначала `GET /api/sessions`, затем хранить выбранный `session_id` и
   использовать scoped REST/WS; legacy URL подходят для single-account клиента.
2. Хранить базовый URL и выбранный id, но не admin token, OTP или Telegram 2FA.
3. Release-клиент должен требовать HTTPS/WSS; cleartext допустим только в
   локальной debug-сборке.
4. У публичного relay API client-auth пока нет — не публиковать его в интернет
   без TLS и VPN/другого сетевого ограничения доступа.
5. Бинарные media и pin mutation остаются только в защищённой admin-панели;
   публичный Android-клиент получает media metadata и `chat_pinned` events.

## Панель аккаунтов

При `LNL_DEBUG_UI=1` отдельный admin-сервер поднимается на
`LNL_ADMIN_BIND` (по умолчанию `127.0.0.1:8081`). Не-loopback адрес
отклоняется. Для запуска нужен `LNL_ADMIN_TOKEN` длиной не менее 32 байт.
Открывай именно напечатанный server URL; с VPS пробрасывай его SSH-туннелем.

Все `/api/admin/*` требуют точный заголовок
`Authorization: Bearer <LNL_ADMIN_TOKEN>`. Изменяющие запросы дополнительно
принимаются только с exact same-origin панели. Ответы авторизации не
кэшируются.

- `GET /api/admin/accounts` — все настроенные слоты, статус и защищённый
  профиль готового аккаунта.
- `POST /api/admin/accounts/{id}/auth/phone` — `{ "phone": "+7999…" }`.
- `POST /api/admin/accounts/{id}/auth/code` —
  `{ "flow_id": "…", "code": "…" }`.
- `POST /api/admin/accounts/{id}/auth/password` —
  `{ "flow_id": "…", "password": "…" }`.
- `GET /api/admin/accounts/{id}/avatar` — безопасный raster-аватар.
- `POST /api/admin/sessions/{id}/messages/{peer_id}/media` — одно
  multipart-вложение до 50 МиБ: `kind=photo|file|voice|video_note`, `file`,
  необязательная `caption`.
- `GET /api/admin/sessions/{id}/messages/{peer_id}/{message_id}/media` —
  защищённое потоковое скачивание; один HTTP byte range поддерживается,
  один ответ ограничен 100 МиБ.
- `PUT|DELETE /api/admin/sessions/{id}/chats/{peer_id}/pin` —
  закрепить или открепить диалог в его текущем списке Telegram.

`flow_id`, OTP и 2FA из одного слота не принимаются другим. OTP, пароль и
Telegram challenge остаются только в памяти. Rich profile и аватар никогда не
добавляются в публичный `GET /api/sessions`.

Бинарные media-route доступны только на loopback admin-сервере и требуют
Bearer. Перед скачиванием сервер повторно получает сообщение из Telegram и
проверяет выбранные session, peer и message id. Имя файла не используется как
путь, загрузка пишется потоково во временный файл с приватными правами.
Фото проверяется по magic bytes. Нативная отправка голосового принимает
OGG/Opus, видеокружка — MP4/H.264 до 60 секунд; длительность, кодек и размеры
сервер читает из файла и не доверяет multipart-метаданным. Релей не выполняет
транскодирование. Статические и WebM-стикеры отображаются inline;
анимированные TGS доступны как защищённое вложение.
