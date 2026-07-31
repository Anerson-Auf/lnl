# Android client (отдельное приложение)

Продуктовый клиент LNL. Сервер — Rust `lnl` в корне репо.

Открой папку `clients/android` в **Android Studio** (Open), дождись Gradle sync, Run на эмулятор/устройство.

## Настройки

- Поле **Relay URL** в приложении (по умолчанию `http://10.0.2.2:8080` — эмулятор → localhost хоста).
- На реальном телефоне: `http://<IP_VPS>:8080` (сервер с `LNL_BIND=0.0.0.0:8080`).
- Контракт: [../../docs/API.md](../../docs/API.md).

## Стек

Kotlin + Compose + OkHttp (REST + WebSocket) + kotlinx.serialization.
