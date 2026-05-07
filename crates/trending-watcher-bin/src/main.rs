//! Trending Watcher CLI — Phase 21'.d (ADR-0059).
//!
//! 본 binary는 GHA cron(6h)가 호출. 흐름:
//! 1. HF Trending(gguf) fetch.
//! 2. Open LLM Leaderboard 2 Parquet fetch (graceful degrade).
//! 3. 1차 join (cards 없이) → top N Review 후보 추출.
//! 4. 후보들의 model card 본문 fetch (Korean signal 강화).
//! 5. 최종 join + score → report.md 작성 (frontmatter + grouping + sort).
//! 6. 후속 GHA step이 JasonEtco/create-an-issue로 Issue 생성/갱신.
//!
//! 외부 통신 화이트리스트 (ADR-0026): `huggingface.co`, `github.com`,
//! `raw.githubusercontent.com`. deterministic 필터 — LLM judge 0.

mod error;
mod fetcher;
mod filter;
mod report;
mod types;

use std::collections::HashMap;

use anyhow::Result;

use crate::fetcher::model_card::fetch_model_card;
use crate::fetcher::{fetch_hf_trending, fetch_leaderboard, hf_trending::make_client};
use crate::filter::{join_candidates, join_candidates_with_cards, Candidate, Queue};

const WATCHER_VERSION: &str = env!("CARGO_PKG_VERSION");
/// HF Trending fetch limit — ADR-0059 §3 권장 (200건 / 6h).
const TRENDING_LIMIT: u32 = 200;
/// model card fetch는 비용 — top N (Review queue 정렬 후 상위)만.
const CARD_FETCH_TOP_N: usize = 30;
/// report.md에 표시할 그룹별 max 항목.
const REPORT_TOP_N: usize = 20;
/// report.md 출력 경로 (workflow_dispatch / cron 둘 다 cwd가 repo root).
const REPORT_PATH: &str = "report.md";

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();
    tracing::info!(version = WATCHER_VERSION, "trending-watcher CLI 시작");

    let dry_run = std::env::var("DRY_RUN")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);

    let client = make_client()?;

    // ---- 1. HF Trending ----
    let trending = match fetch_hf_trending(&client, TRENDING_LIMIT, Some("gguf")).await {
        Ok(t) => {
            tracing::info!(count = t.len(), "HF Trending fetch 성공");
            if t.is_empty() {
                tracing::warn!("HF Trending 응답이 비었어요 — 후속 단계 skip");
                return Ok(());
            }
            t
        }
        Err(e) => {
            tracing::error!(error = %e, "HF Trending fetch 실패");
            return Err(e.into());
        }
    };

    // ---- 2. Open LLM Leaderboard 2 ----
    let leaderboard = match fetch_leaderboard(&client).await {
        Ok(lb) => {
            tracing::info!(count = lb.len(), "Open LLM Leaderboard 2 fetch 성공");
            lb
        }
        Err(e) => {
            tracing::warn!(error = %e, "Leaderboard fetch 실패 — trending only 진행");
            Vec::new()
        }
    };

    // ---- 3. 1차 join (cards 없이) → top N Review 추출 ----
    let pre_candidates = join_candidates(&trending, &leaderboard);
    let mut top_review: Vec<&Candidate> = pre_candidates
        .iter()
        .filter(|c| c.queue == Queue::Review)
        .collect();
    top_review.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let top_ids: Vec<String> = top_review
        .iter()
        .take(CARD_FETCH_TOP_N)
        .map(|c| c.id.clone())
        .collect();

    // ---- 4. model card fetch (top N만) ----
    tracing::info!(
        top_n = top_ids.len(),
        "model card fetch 시작 (Korean signal 강화용)"
    );
    let mut cards: HashMap<String, String> = HashMap::new();
    for id in &top_ids {
        match fetch_model_card(&client, id).await {
            Ok(Some(body)) => {
                cards.insert(id.clone(), body);
            }
            Ok(None) => {
                tracing::debug!(id = %id, "README 없음 — graceful");
            }
            Err(e) => {
                tracing::warn!(error = %e, id = %id, "card fetch 실패 — 다음 모델로 진행");
            }
        }
    }
    tracing::info!(
        fetched = cards.len(),
        attempted = top_ids.len(),
        "model card fetch 완료"
    );

    // ---- 5. 최종 join with cards + report ----
    let final_candidates = join_candidates_with_cards(&trending, &leaderboard, &cards);

    let review_count = final_candidates
        .iter()
        .filter(|c| c.queue == Queue::Review)
        .count();
    let info_count = final_candidates
        .iter()
        .filter(|c| c.queue == Queue::InfoOnly)
        .count();
    let excl_count = final_candidates
        .iter()
        .filter(|c| c.queue == Queue::Excluded)
        .count();
    tracing::info!(
        review = review_count,
        info_only = info_count,
        excluded = excl_count,
        "최종 candidates 분류 완료"
    );

    if dry_run {
        tracing::info!("DRY_RUN=true — report.md 작성 skip");
        // 콘솔 preview 5건.
        let mut sorted: Vec<&Candidate> = final_candidates
            .iter()
            .filter(|c| c.queue == Queue::Review)
            .collect();
        sorted.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        for c in sorted.iter().take(5) {
            tracing::info!(
                id = %c.id,
                score = format!("{:.3}", c.score),
                "preview"
            );
        }
        return Ok(());
    }

    let report_text = report::generate_report(&final_candidates, REPORT_TOP_N);
    std::fs::write(REPORT_PATH, &report_text)?;
    tracing::info!(
        path = REPORT_PATH,
        bytes = report_text.len(),
        "report.md 작성 완료 — 후속 GHA step이 Issue로 publish"
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
