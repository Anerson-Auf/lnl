# LNL Relay API (для Android)

База: `http://<host>:8080` (или `https://…` за nginx).  
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

## Рекомендации Android

1. Базовый URL в настройках (не хардкод прод-IP в release без экрана настроек).
2. Retrofit/OkHttp + OkHttp WebSocket (или Ktor).
3. Список чатов → экран диалога → `POST` + подписка на WS для live.
4. На cleartext HTTP в debug: `android:usesCleartextTraffic="true"` или network security config.
5. Сначала `GET /api/sessions`, затем хранить выбранный `session_id` и
   использовать scoped REST/WS; legacy URL подходят для single-account клиента.
6. Auth пока нет — не светить API в интернет без токена/VPN. При нескольких
   аккаунтах цена ошибочной публикации API выше.
