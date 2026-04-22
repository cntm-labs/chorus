use axum::middleware::Next;
use axum::response::IntoResponse;
use tracing::Instrument;
use uuid::Uuid;

/// Middleware that generates a unique request ID and wraps the request in a tracing span.
pub async fn inject(request: axum::extract::Request, next: Next) -> impl IntoResponse {
    let request_id = request
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_owned())
        .unwrap_or_else(|| Uuid::now_v7().to_string());

    let span = tracing::info_span!(
        "request",
        request_id = %request_id,
        method = %request.method(),
        path = %request.uri().path(),
    );

    let mut response = next.run(request).instrument(span).await;

    response.headers_mut().insert(
        "x-request-id",
        request_id.parse().expect("valid header value"),
    );

    response
}
