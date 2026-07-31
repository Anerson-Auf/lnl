# Android-клиент LNL

Отдельное Compose-приложение для публичного relay API из корня репозитория.

## Что поддерживается

- выбор любого готового Telegram-аккаунта из `GET /api/sessions`;
- сохранение Relay URL и выбранного аккаунта между запусками;
- системные светлая/тёмная темы, быстрые разделы чатов, аккаунтов и Relay;
- локальный поиск по диалогам и фильтр закреплённых чатов;
- scoped-диалоги, история, отправка текста и live WebSocket выбранной сессии;
- отображение фото, файлов, стикеров, аудио, видео, голосовых и видеокружков по metadata;
- live-состояние закреплённых диалогов;
- отмена устаревших запросов при смене аккаунта/чата и reconnect с backoff.

Приложение не содержит тестовых чатов. Если аккаунтов нет, сначала авторизуй
Telegram-сессию в защищённой web-панели relay.

## Запуск

Открой `clients/android` в Android Studio или выполни:

```sh
./gradlew testDebugUnitTest lintDebug assembleDebug
```

Debug-сборка по умолчанию подключается к `http://10.0.2.2:8080` — это
`localhost` хоста из Android Emulator. Для физического устройства укажи URL
relay вручную.

Release-сборка принимает только HTTPS/WSS. Cleartext разрешён только в debug,
а Android backup отключён. Relay URL и id выбранной сессии сохраняются в
обычных `SharedPreferences`; секреты клиент не хранит.

## Граница public/admin

Android использует только публичные scoped-маршруты `:8080`. Admin token в APK
не встраивается. Авторизация аккаунтов, аватары, бинарная отправка/скачивание
медиа и изменение закрепов остаются на loopback-only admin API `:8081` с
Bearer и exact-origin проверкой. Android отображает пришедшие через public API
media metadata и pin events, но не вызывает admin-маршруты. Поэтому круглые
аватары в списках строятся локально из инициалов и не выдаются за профильные
фотографии Telegram.

У публичного API пока нет client-auth. Не публикуй `:8080` напрямую в интернет:
используй TLS reverse proxy вместе с VPN/другим сетевым ограничением доступа.

Полный контракт: [docs/API.md](../../docs/API.md).

## Стек

Kotlin, Jetpack Compose, OkHttp REST/WebSocket, kotlinx.serialization и
kotlinx.coroutines.
