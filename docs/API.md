# LNL Relay API (для Android)

База: `http://<host>:8080` (или `https://…` за nginx).  
`peer_id` — Bot API id (`i64`).

## REST

### `GET /api/chats`

Список чатов из папки релея.

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

История (старые → новые).

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

Сервер пушит JSON:

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
5. Auth пока нет — не светить API в интернет без токена/VPN.
