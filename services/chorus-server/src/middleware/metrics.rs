use axum::extract::MatchedPath;
use axum::middleware::Next;
use axum::response::IntoResponse;
use std::time::Instant;

/// Middleware that records HTTP request duration and total count as Prometheus metrics.
pub async fn track(request: axum::extract::Request, next: Next) -> impl IntoResponse {
    let path = request
        .extensions()
        .get::<MatchedPath>()
        .map(|p| p.as_str().to_owned())
        .unwrap_or_else(|| request.uri().path().to_owned());
    let method = request.method().to_string();

    let start = Instant::now();
    let response = next.run(request).await;
    let duration = start.elapsed().as_secs_f64();
    let status = response.status().as_u16().to_string();

    metrics::histogram!(
        "chorus_http_request_duration_seconds",
        "method" => method.clone(),
        "path" => path.clone(),
        "status" => status.clone(),
    )
    .record(duration);

    metrics::counter!(
        "chorus_http_requests_total",
        "method" => method,
        "path" => path,
        "status" => status,
    )
    .increment(1);

    response
}
