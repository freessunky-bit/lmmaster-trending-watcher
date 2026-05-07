//! HuggingFace `/api/models?sort=trending` fetcher — Phase 21'.b (ADR-0059 §3).
//!
//! 정책:
//! - User-Agent 명시 — `lmmaster-trending-watcher/<ver>`.
//! - 429 응답 시 `RateLimit-Retry-After` (또는 `Retry-After`) 헤더 honor.
//! - timeout 30s + rustls (no proxy).
//! - sort=trending + library=gguf 옵션은 caller가 결정 — 본 함수는 *기본 sort=trending*.
//! - schema는 *부분만* deserialize (TrendingModelMeta) — schema drift 영향 최소화.

#![allow(dead_code)]

use crate::error::{WatcherError, WatcherResult};
use crate::types::TrendingModelMeta;

pub const HF_TRENDING_ENDPOINT: &str = "https://huggingface.co/api/models";

const USER_AGENT: &str = concat!("lmmaster-trending-watcher/", env!("CARGO_PKG_VERSION"));

/// HF Trending API 호출 — `?sort=trending&limit={limit}` + optional library filter.
///
/// 응답:
/// - HTTP 200 + JSON 배열 → `Vec<TrendingModelMeta>`.
/// - HTTP 429 → `Err(RateLimited { retry_after_secs })`.
/// - HTTP 4xx/5xx → `Err(HfApiUnreachable)`.
/// - 빈 배열 → `Ok(vec![])` (graceful — caller가 warning).
pub async fn fetch_hf_trending(
    client: &reqwest::Client,
    limit: u32,
    library_filter: Option<&str>,
) -> WatcherResult<Vec<TrendingModelMeta>> {
    fetch_hf_trending_with_base(client, HF_TRENDING_ENDPOINT, limit, library_filter).await
}

/// 테스트용 — base URL 주입 (wiremock).
pub async fn fetch_hf_trending_with_base(
    client: &reqwest::Client,
    base: &str,
    limit: u32,
    library_filter: Option<&str>,
) -> WatcherResult<Vec<TrendingModelMeta>> {
    // HF API sort 값: trendingScore (또는 downloads / lastModified). `trending`은 deprecated.
    let mut url = format!("{base}?sort=trendingScore&limit={limit}");
    if let Some(lib) = library_filter {
        url.push_str("&library=");
        url.push_str(lib);
    }

    let resp = client
        .get(&url)
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .send()
        .await
        .map_err(|e| WatcherError::HfApiUnreachable(format!("{url}: {e}")))?;

    if resp.status() == 429 {
        let retry_after = resp
            .headers()
            .get("RateLimit-Retry-After")
            .or_else(|| resp.headers().get("Retry-After"))
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse().ok())
            .unwrap_or(60);
        return Err(WatcherError::RateLimited {
            retry_after_secs: retry_after,
        });
    }

    if !resp.status().is_success() {
        return Err(WatcherError::HfApiUnreachable(format!(
            "HTTP {} for {url}",
            resp.status()
        )));
    }

    let body = resp
        .text()
        .await
        .map_err(|e| WatcherError::HfApiUnreachable(format!("body read: {e}")))?;

    parse_trending_response(&body)
}

/// JSON 응답 파싱 — 단위 테스트 가능.
///
/// 응답 schema가 *배열*이 아니면 `SchemaMismatch`. 배열 안 row가 *partial parse 실패*면
/// graceful skip (warning 로깅 후 skip) — 한 row schema drift가 전체 fetch를 깨뜨리지 않게.
pub fn parse_trending_response(json: &str) -> WatcherResult<Vec<TrendingModelMeta>> {
    let raw: Vec<serde_json::Value> = serde_json::from_str(json).map_err(|e| {
        WatcherError::SchemaMismatch(format!(
            "응답이 array가 아니에요: {e} ({})",
            &json.chars().take(120).collect::<String>()
        ))
    })?;

    let mut out = Vec::with_capacity(raw.len());
    for v in raw {
        match serde_json::from_value::<TrendingModelMeta>(v.clone()) {
            Ok(m) => out.push(m),
            Err(e) => {
                tracing::warn!(error = %e, raw = %v, "한 row 파싱 실패 — skip");
            }
        }
    }
    Ok(out)
}

/// `make_client` — `.no_proxy()` + rustls + 30s timeout. ADR-0026 외부 통신 정책 정합.
pub fn make_client() -> WatcherResult<reqwest::Client> {
    reqwest::Client::builder()
        .no_proxy()
        .user_agent(USER_AGENT)
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| WatcherError::Internal(format!("reqwest::Client builder: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// **invariant 1** — schema parsing: known good fixture → Vec.
    #[test]
    fn parse_two_models_basic() {
        let json = r#"[
            {"id": "Qwen/Qwen2.5-7B-Instruct", "downloads": 1000, "tags": ["text-generation", "ko"]},
            {"id": "elyza/Llama-3-ELYZA-JP-8B", "downloads": 500, "tags": ["text-generation"]}
        ]"#;
        let v = parse_trending_response(json).unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].id, "Qwen/Qwen2.5-7B-Instruct");
        assert!(v[0].has_korean_tag());
        assert!(!v[1].has_korean_tag());
    }

    /// **invariant 2** — empty fetch: 빈 배열 → graceful Ok(empty).
    #[test]
    fn parse_empty_array_graceful() {
        let v = parse_trending_response("[]").unwrap();
        assert!(v.is_empty());
    }

    /// **invariant 3** — schema mismatch: array가 아니면 Err.
    #[test]
    fn parse_object_returns_schema_mismatch() {
        let err = parse_trending_response(r#"{"not": "an array"}"#).unwrap_err();
        assert!(matches!(err, WatcherError::SchemaMismatch(_)));
    }

    /// **invariant 4** — partial parse failure: 한 row만 깨져도 나머지는 보존.
    #[test]
    fn parse_drops_invalid_row_keeps_rest() {
        let json = r#"[
            {"id": "ok/model", "downloads": 100},
            {"this_is_not_a_model": true},
            {"id": "ok/model2", "downloads": 200}
        ]"#;
        let v = parse_trending_response(json).unwrap();
        // schema drift row는 partial parse 성공할 수도 (id 누락이지만 downloads도 누락 → default).
        // 핵심: 적어도 *valid 2개*는 보존 (drop 0~1개).
        assert!(v.len() >= 2, "valid rows must be preserved");
        assert!(v.iter().any(|m| m.id == "ok/model"));
        assert!(v.iter().any(|m| m.id == "ok/model2"));
    }

    /// **invariant 5** — 429 handling: RateLimit-Retry-After honor.
    #[tokio::test]
    async fn fetch_429_returns_rate_limited_with_retry_after() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/models"))
            .respond_with(ResponseTemplate::new(429).insert_header("RateLimit-Retry-After", "120"))
            .mount(&server)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let base = format!("{}/api/models", server.uri());
        let err = fetch_hf_trending_with_base(&client, &base, 200, None)
            .await
            .unwrap_err();
        match err {
            WatcherError::RateLimited { retry_after_secs } => {
                assert_eq!(retry_after_secs, 120);
            }
            other => panic!("expected RateLimited, got {other:?}"),
        }
    }

    /// **invariant 6** — query param: limit + library 정확히 전송.
    #[tokio::test]
    async fn fetch_sends_sort_limit_library_query() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/models"))
            .and(query_param("sort", "trendingScore"))
            .and(query_param("limit", "50"))
            .and(query_param("library", "gguf"))
            .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
            .mount(&server)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let base = format!("{}/api/models", server.uri());
        let v = fetch_hf_trending_with_base(&client, &base, 50, Some("gguf"))
            .await
            .unwrap();
        assert!(v.is_empty());
    }

    /// **invariant 7** — 5xx 에러: HfApiUnreachable.
    #[tokio::test]
    async fn fetch_500_returns_unreachable() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/models"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let base = format!("{}/api/models", server.uri());
        let err = fetch_hf_trending_with_base(&client, &base, 200, None)
            .await
            .unwrap_err();
        assert!(matches!(err, WatcherError::HfApiUnreachable(_)));
    }
}
