//! HuggingFace model card 본문 fetcher — Phase 21'.c.3 (ADR-0059 §3).
//!
//! 정책:
//! - endpoint: `https://huggingface.co/{id}/raw/main/README.md` — markdown 본문 RAW.
//! - User-Agent + 30s timeout + rustls (no proxy).
//! - 404 → graceful `Ok(None)` (README 없는 모델은 정상 케이스).
//! - 429 → `RateLimited` (다른 fetcher와 동형).
//! - 5xx / 기타 4xx → `HfApiUnreachable`.

#![allow(dead_code)]

use crate::error::{WatcherError, WatcherResult};

pub const HF_RAW_BASE: &str = "https://huggingface.co";

const USER_AGENT: &str = concat!("lmmaster-trending-watcher/", env!("CARGO_PKG_VERSION"));

/// HuggingFace model card (`README.md`) 본문 fetch. README 없는 모델은 `Ok(None)`.
pub async fn fetch_model_card(client: &reqwest::Client, id: &str) -> WatcherResult<Option<String>> {
    fetch_model_card_with_base(client, HF_RAW_BASE, id).await
}

/// 테스트용 — base URL 주입 (wiremock).
pub async fn fetch_model_card_with_base(
    client: &reqwest::Client,
    base: &str,
    id: &str,
) -> WatcherResult<Option<String>> {
    let url = format!("{base}/{id}/raw/main/README.md");
    let resp = client
        .get(&url)
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .send()
        .await
        .map_err(|e| WatcherError::HfApiUnreachable(format!("{url}: {e}")))?;

    if resp.status() == 404 {
        // README 없는 모델은 정상 케이스 — graceful.
        return Ok(None);
    }
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
    Ok(Some(body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// **invariant 1** — 200 + body → Ok(Some(body)).
    #[tokio::test]
    async fn fetch_returns_body_on_200() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/Qwen/Qwen2.5-7B-Instruct/raw/main/README.md"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("# Qwen2.5\n\n이 모델은 한국어 추론에 강력해요."),
            )
            .mount(&server)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let body = fetch_model_card_with_base(&client, &server.uri(), "Qwen/Qwen2.5-7B-Instruct")
            .await
            .unwrap();
        assert!(body.is_some());
        assert!(body.unwrap().contains("한국어"));
    }

    /// **invariant 2** — 404 → Ok(None) (graceful).
    #[tokio::test]
    async fn fetch_404_returns_none() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/missing/repo/raw/main/README.md"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let body = fetch_model_card_with_base(&client, &server.uri(), "missing/repo")
            .await
            .unwrap();
        assert!(body.is_none(), "404 → graceful None");
    }

    /// **invariant 3** — 429 → RateLimited (RateLimit-Retry-After honor).
    #[tokio::test]
    async fn fetch_429_returns_rate_limited() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/x/y/raw/main/README.md"))
            .respond_with(ResponseTemplate::new(429).insert_header("RateLimit-Retry-After", "45"))
            .mount(&server)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let err = fetch_model_card_with_base(&client, &server.uri(), "x/y")
            .await
            .unwrap_err();
        match err {
            WatcherError::RateLimited { retry_after_secs } => {
                assert_eq!(retry_after_secs, 45);
            }
            other => panic!("expected RateLimited, got {other:?}"),
        }
    }

    /// **invariant 4** — 500 → HfApiUnreachable.
    #[tokio::test]
    async fn fetch_500_returns_unreachable() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/x/y/raw/main/README.md"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let err = fetch_model_card_with_base(&client, &server.uri(), "x/y")
            .await
            .unwrap_err();
        assert!(matches!(err, WatcherError::HfApiUnreachable(_)));
    }
}
