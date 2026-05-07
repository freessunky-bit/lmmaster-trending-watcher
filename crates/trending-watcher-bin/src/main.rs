//! Trending Watcher CLI — Phase 21'.a (ADR-0059).
//!
//! 본 binary는 21'.a 단계에서 *골격*만 — fetcher / filter / report 모듈은 21'.b ~ 21'.d에서.
//!
//! 정책:
//! - 외부 통신 화이트리스트: `huggingface.co`, `github.com`, `raw.githubusercontent.com`만.
//! - deterministic 필터 — LLM judge 0. 가중치 매트릭스는 ADR-0059 §4.
//! - GHA cron 6h가 본 binary를 호출. 결과는 `report.md` + 후속 GHA step이 GitHub Issue 생성.

use anyhow::Result;

const WATCHER_VERSION: &str = env!("CARGO_PKG_VERSION");

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    tracing::info!(version = WATCHER_VERSION, "trending-watcher CLI 시작");

    // 21'.a 단계 — 골격만 (fetcher / filter는 후속).
    tracing::info!("Phase 21'.a — fetcher / filter / report 모듈은 21'.b~.d에서 합류해요.");
    tracing::info!(
        "외부 통신 화이트리스트: huggingface.co, github.com, raw.githubusercontent.com."
    );

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
