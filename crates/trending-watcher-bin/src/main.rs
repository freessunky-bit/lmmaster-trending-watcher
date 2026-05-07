//! Trending Watcher CLI — Phase 21'.b (ADR-0059).
//!
//! 본 binary는 21'.b에서 *HF Trending fetch까지*. filter / report 모듈은 21'.c ~ 21'.d에서.
//!
//! 정책:
//! - 외부 통신 화이트리스트: `huggingface.co`, `github.com`, `raw.githubusercontent.com`만.
//! - deterministic 필터 — LLM judge 0. 가중치 매트릭스는 ADR-0059 §4.
//! - GHA cron 6h가 본 binary를 호출. 결과는 `report.md` + 후속 GHA step이 GitHub Issue 생성.

mod error;
mod fetcher;
mod types;

use anyhow::Result;

use crate::fetcher::{fetch_hf_trending, fetch_leaderboard, hf_trending::make_client};

const WATCHER_VERSION: &str = env!("CARGO_PKG_VERSION");
const DEFAULT_LIMIT: u32 = 200;

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    tracing::info!(version = WATCHER_VERSION, "trending-watcher CLI 시작");

    let dry_run = std::env::var("DRY_RUN")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);

    let client = make_client()?;

    // HF Trending fetch — 21'.b cut 1.
    match fetch_hf_trending(&client, DEFAULT_LIMIT, Some("gguf")).await {
        Ok(models) => {
            tracing::info!(
                count = models.len(),
                "HF Trending(gguf, sort=trending, limit=200) fetch 성공"
            );
            if models.is_empty() {
                tracing::warn!("HF Trending 응답이 비었어요 — graceful skip");
            }
            // dry_run 모드에서 처음 5개 id 로깅.
            if dry_run {
                for m in models.iter().take(5) {
                    tracing::info!(id = %m.id, downloads = m.downloads, "preview");
                }
            }
        }
        Err(e) => {
            tracing::error!(error = %e, "HF Trending fetch 실패");
            return Err(e.into());
        }
    }

    // Open LLM Leaderboard 2 fetch — 21'.b.2 cut.
    match fetch_leaderboard(&client).await {
        Ok(entries) => {
            tracing::info!(
                count = entries.len(),
                "Open LLM Leaderboard 2 (open-llm-leaderboard/contents) fetch 성공"
            );
            if entries.is_empty() {
                tracing::warn!("Leaderboard 응답이 비었어요 — graceful skip");
            }
            if dry_run {
                for e in entries.iter().take(5) {
                    tracing::info!(
                        eval_name = %e.eval_name,
                        average = e.average,
                        "leaderboard preview"
                    );
                }
            }
        }
        Err(e) => {
            // Leaderboard fetch 실패가 watcher 전체를 깨뜨리지는 않음 — HF API 장애 여전히
            // gracefully degrade하고 trending만으로 진행 가능하게 warning만 emit.
            tracing::warn!(error = %e, "Leaderboard fetch 실패 — trending only 진행");
        }
    }

    tracing::info!("Phase 21'.c (filter) + 21'.d (report + Issue) 모듈은 후속 sub-phase에서.");

    Ok(())
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,trending_watcher=debug"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .json()
        .try_init();
}

#[cfg(test)]
mod tests {
    #[test]
    fn watcher_version_is_set() {
        assert!(!super::WATCHER_VERSION.is_empty());
        assert!(super::WATCHER_VERSION.starts_with("0."));
    }
}
