use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::header::{AUTHORIZATION, CACHE_CONTROL, CONTENT_TYPE, ETAG, ORIGIN, RETRY_AFTER};
use axum::http::{HeaderMap, HeaderValue, Method, Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;

use crate::tg::accounts::{AccountError, AccountErrorKind, AccountManager, AuthReply};

#[derive(Clone)]
pub struct AdminToken(Arc<Vec<u8>>);

impl AdminToken {
    pub fn parse(raw: String) -> Result<Self, String> {
        if raw.len() < 32 {
            return Err("LNL_ADMIN_TOKEN должен содержать минимум 32 байта".to_string());
        }
        if raw.len() > 256 || raw.chars().any(char::is_whitespace) {
            return Err("LNL_ADMIN_TOKEN имеет недопустимый формат".to_string());
        }
        Ok(Self(Arc::new(raw.into_bytes())))
    }

    fn matches(&self, candidate: &[u8]) -> bool {
        candidate.len() == self.0.len() && bool::from(candidate.ct_eq(self.0.as_slice()))
    }
}

#[derive(Clone)]
pub struct AdminAccess {
    token: AdminToken,
    origin: Arc<str>,
}

impl AdminAccess {
    pub fn new(token: AdminToken, origin: String) -> Self {
        Self {
            token,
            origin: Arc::from(origin),
        }
    }
}

pub fn validate_admin_bind(bind: SocketAddr) -> Result<(), String> {
    if !bind.ip().is_loopback() {
        return Err(format!(
            "LNL_ADMIN_BIND={bind}: панель авторизации разрешена только на loopback"
        ));
    }
    Ok(())
}

pub fn router(manager: Arc<AccountManager>, access: AdminAccess) -> Router {
    Router::new()
        .route("/api/admin/accounts", get(list_accounts))
        .route(
            "/api/admin/accounts/{account_id}/avatar",
            get(account_avatar),
        )
        .route(
            "/api/admin/accounts/{account_id}/auth/phone",
            post(auth_phone),
        )
        .route(
            "/api/admin/accounts/{account_id}/auth/code",
            post(auth_code),
        )
        .route(
            "/api/admin/accounts/{account_id}/auth/password",
            post(auth_password),
        )
        .route_layer(middleware::from_fn_with_state(access, require_admin))
        .layer(DefaultBodyLimit::max(4096))
        .with_state(manager)
}

pub async fn require_admin(
    State(access): State<AdminAccess>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if !authorized(request.headers(), &access.token) {
        return admin_denied(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    if is_unsafe(request.method()) && !same_origin(request.headers(), &access.origin) {
        return admin_denied(StatusCode::FORBIDDEN, "origin_denied");
    }

    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn authorized(headers: &HeaderMap, token: &AdminToken) -> bool {
    let mut values = headers.get_all(AUTHORIZATION).iter();
    let Some(value) = values.next() else {
        return false;
    };
    if values.next().is_some() {
        return false;
    }
    let Ok(value) = value.to_str() else {
        return false;
    };
    let Some(candidate) = value.strip_prefix("Bearer ") else {
        return false;
    };
    !candidate.is_empty() && token.matches(candidate.as_bytes())
}

fn same_origin(headers: &HeaderMap, expected: &str) -> bool {
    let mut values = headers.get_all(ORIGIN).iter();
    let Some(value) = values.next() else {
        return false;
    };
    if values.next().is_some() {
        return false;
    }
    value.as_bytes() == expected.as_bytes()
}

fn is_unsafe(method: &Method) -> bool {
    !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
}

#[derive(Serialize)]
struct AdminErrorBody {
    error: &'static str,
    code: &'static str,
}

fn admin_denied(status: StatusCode, code: &'static str) -> Response {
    (
        status,
        [(CACHE_CONTROL, "no-store")],
        Json(AdminErrorBody {
            error: "Доступ запрещён",
            code,
        }),
    )
        .into_response()
}

async fn list_accounts(State(manager): State<Arc<AccountManager>>) -> impl IntoResponse {
    Json(manager.summaries().await)
}

async fn account_avatar(
    State(manager): State<Arc<AccountManager>>,
    Path(account_id): Path<String>,
) -> Response {
    match manager.avatar(&account_id).await {
        Ok(avatar) => {
            let mut response = Response::new(Body::from((*avatar.bytes).clone()));
            response
                .headers_mut()
                .insert(CONTENT_TYPE, HeaderValue::from_static(avatar.content_type));
            response
                .headers_mut()
                .insert(CACHE_CONTROL, HeaderValue::from_static("private, no-store"));
            response.headers_mut().insert(
                "x-content-type-options",
                HeaderValue::from_static("nosniff"),
            );
            if let Ok(etag) = HeaderValue::from_str(&format!("\"{}\"", avatar.version)) {
                response.headers_mut().insert(ETAG, etag);
            }
            response
        }
        Err(error) => account_error(error),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PhoneBody {
    phone: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CodeBody {
    flow_id: String,
    code: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PasswordBody {
    flow_id: String,
    password: String,
}

async fn auth_phone(
    State(manager): State<Arc<AccountManager>>,
    Path(account_id): Path<String>,
    Json(body): Json<PhoneBody>,
) -> Result<Json<AuthReply>, Response> {
    manager
        .request_phone(&account_id, &body.phone)
        .await
        .map(Json)
        .map_err(account_error)
}

async fn auth_code(
    State(manager): State<Arc<AccountManager>>,
    Path(account_id): Path<String>,
    Json(body): Json<CodeBody>,
) -> Result<Json<AuthReply>, Response> {
    manager
        .submit_code(&account_id, &body.flow_id, &body.code)
        .await
        .map(Json)
        .map_err(account_error)
}

async fn auth_password(
    State(manager): State<Arc<AccountManager>>,
    Path(account_id): Path<String>,
    Json(body): Json<PasswordBody>,
) -> Result<Json<AuthReply>, Response> {
    manager
        .submit_password(&account_id, &body.flow_id, &body.password)
        .await
        .map(Json)
        .map_err(account_error)
}

fn account_error(error: AccountError) -> Response {
    let status = match error.kind {
        AccountErrorKind::NotFound => StatusCode::NOT_FOUND,
        AccountErrorKind::BadInput => StatusCode::BAD_REQUEST,
        AccountErrorKind::Conflict => StatusCode::CONFLICT,
        AccountErrorKind::Expired => StatusCode::GONE,
        AccountErrorKind::RateLimited => StatusCode::TOO_MANY_REQUESTS,
        AccountErrorKind::Telegram => StatusCode::BAD_GATEWAY,
        AccountErrorKind::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    };
    let mut response = (
        status,
        Json(AdminErrorBody {
            error: error.message,
            code: error.code,
        }),
    )
        .into_response();
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if let Some(seconds) = error.retry_after
        && let Ok(value) = HeaderValue::from_str(&seconds.to_string())
    {
        response.headers_mut().insert(RETRY_AFTER, value);
    }
    response
}

pub async fn security_headers(request: Request<Body>, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        "content-security-policy",
        HeaderValue::from_static(
            "default-src 'self'; img-src 'self' blob: data:; media-src 'self' blob:; \
             object-src 'none'; connect-src 'self' ws: wss:; \
             style-src 'self' 'unsafe-inline'; script-src 'self' 'unsafe-inline'; \
             base-uri 'none'; frame-ancestors 'none'; form-action 'self'",
        ),
    );
    headers.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert("x-frame-options", HeaderValue::from_static("DENY"));
    response
}

#[cfg(test)]
mod tests {
    use super::{AdminAccess, AdminToken, require_admin, validate_admin_bind};
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::middleware;
    use axum::routing::get;
    use tower::ServiceExt;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef";
    const ORIGIN: &str = "http://127.0.0.1:9081";

    fn protected() -> Router {
        let access = AdminAccess::new(
            AdminToken::parse(TOKEN.to_string()).unwrap(),
            ORIGIN.to_string(),
        );
        Router::new()
            .route("/probe", get(|| async { "ok" }).post(|| async { "ok" }))
            .route_layer(middleware::from_fn_with_state(access, require_admin))
    }

    #[tokio::test]
    async fn exact_bearer_is_required() {
        for value in [
            None,
            Some("Basic abc"),
            Some("Bearer"),
            Some("Bearer wrong"),
            Some("bearer 0123456789abcdef0123456789abcdef"),
        ] {
            let mut request = Request::get("/probe");
            if let Some(value) = value {
                request = request.header("authorization", value);
            }
            let response = protected()
                .oneshot(request.body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }

        let response = protected()
            .oneshot(
                Request::get("/probe")
                    .header("authorization", format!("Bearer {TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let response = protected()
            .oneshot(
                Request::get("/probe")
                    .header("authorization", format!("Bearer {TOKEN}"))
                    .header("authorization", format!("Bearer {TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn mutations_require_the_exact_origin() {
        for origin in [
            None,
            Some("null"),
            Some("http://127.0.0.1:90810"),
            Some("https://127.0.0.1:9081"),
            Some("http://evil.example"),
        ] {
            let mut request =
                Request::post("/probe").header("authorization", format!("Bearer {TOKEN}"));
            if let Some(origin) = origin {
                request = request.header("origin", origin);
            }
            let response = protected()
                .oneshot(request.body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
        }

        let response = protected()
            .oneshot(
                Request::post("/probe")
                    .header("authorization", format!("Bearer {TOKEN}"))
                    .header("origin", ORIGIN)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let response = protected()
            .oneshot(
                Request::post("/probe")
                    .header("authorization", format!("Bearer {TOKEN}"))
                    .header("origin", ORIGIN)
                    .header("origin", ORIGIN)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[test]
    fn admin_listener_is_loopback_only() {
        assert!(validate_admin_bind("127.0.0.1:0".parse().unwrap()).is_ok());
        assert!(validate_admin_bind("[::1]:0".parse().unwrap()).is_ok());
        assert!(validate_admin_bind("0.0.0.0:9081".parse().unwrap()).is_err());
        assert!(validate_admin_bind("[::]:9081".parse().unwrap()).is_err());
        assert!(validate_admin_bind("192.168.1.5:9081".parse().unwrap()).is_err());
    }

    #[test]
    fn admin_token_has_a_strong_bounded_format() {
        assert!(AdminToken::parse("short".to_string()).is_err());
        assert!(AdminToken::parse(TOKEN.to_string()).is_ok());
        assert!(AdminToken::parse(format!("{TOKEN}\n")).is_err());
        assert!(AdminToken::parse("x".repeat(257)).is_err());
    }
}
