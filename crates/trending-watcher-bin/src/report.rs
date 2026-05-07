//! Vec<Candidate> → GitHub Issue용 markdown report — Phase 21'.d (ADR-0059 §6).
//!
//! 정책:
//! - frontmatter: title / labels / assignees — JasonEtco/create-an-issue@v2 호환.
//! - 그룹별 헤더 (Review / Info-only / Excluded) + score 내림차순 정렬.
//! - top_n 제한 — 큐레이터가 한 번에 검토 가능한 분량 (기본 20).
//! - GENERATED_AT은 GHA workflow의 `{{ env.GENERATED_AT }}` Liquid 템플릿으로 inject.

#![allow(dead_code)]

use std::cmp::Ordering;
use std::fmt::Write as _;

use crate::filter::{Candidate, Queue};

/// JasonEtco/create-an-issue 호환 markdown — 첫 줄부터 frontmatter.
pub fn generate_report(candidates: &[Candidate], top_n: usize) -> String {
    let mut review: Vec<&Candidate> = candidates
        .iter()
        .filter(|c| c.queue == Queue::Review)
        .collect();
    let mut info_only: Vec<&Candidate> = candidates
        .iter()
        .filter(|c| c.queue == Queue::InfoOnly)
        .collect();
    let mut excluded: Vec<&Candidate> = candidates
        .iter()
        .filter(|c| c.queue == Queue::Excluded)
        .collect();

    let by_score_desc =
        |a: &&Candidate, b: &&Candidate| b.score.partial_cmp(&a.score).unwrap_or(Ordering::Equal);
    review.sort_by(by_score_desc);
    info_only.sort_by(by_score_desc);
    excluded.sort_by(by_score_desc);

    let mut s = String::new();
    // frontmatter — JasonEtco가 title/labels/assignees 자동 추출.
    s.push_str("---\n");
    s.push_str("title: \"Trending Watcher — 큐레이션 review queue\"\n");
    s.push_str("labels: auto-curate, trending-watcher, needs-review\n");
    s.push_str("assignees: freessunky-bit\n");
    s.push_str("---\n\n");

    s.push_str(
        "> 자동 발견된 후보예요. 큐레이터가 검토 후 LMmaster 본 repo에 manifest PR을 올려 주세요.\n",
    );
    s.push_str("> 검토 체크리스트는 [CURATION_GUIDE.md](../CURATION_GUIDE.md) 참고.\n\n");
    // JasonEtco는 Nunjucks templating 사용 — Liquid filter 문법(`| default: "..."`) 비호환.
    // GENERATED_AT은 cron.yml step에서 무조건 set이라 default 불필요.
    s.push_str("생성: {{ env.GENERATED_AT }}\n\n");

    let _ = writeln!(
        s,
        "## Review queue — {} 건 (사이즈 3~14B + license 통과)\n",
        review.len()
    );
    if review.is_empty() {
        s.push_str("이번 cron에서 정식 큐 후보가 없어요.\n\n");
    } else {
        for (i, c) in review.iter().take(top_n).enumerate() {
            write_candidate_block(&mut s, c, i + 1);
        }
    }

    let _ = writeln!(s, "## Info-only — {} 건 (사이즈 외이)\n", info_only.len());
    if info_only.is_empty() {
        s.push_str("(없음)\n\n");
    } else {
        for (i, c) in info_only.iter().take(top_n).enumerate() {
            write_candidate_block(&mut s, c, i + 1);
        }
    }

    let _ = writeln!(
        s,
        "## Excluded — {} 건 (license 화이트리스트 미통과)\n",
        excluded.len()
    );
    if excluded.is_empty() {
        s.push_str("(없음)\n\n");
    } else {
        s.push_str(
            "자동 제외된 후보 ID만 나열해요. 라이선스 매핑 갱신 시 다시 보일 수 있어요.\n\n",
        );
        for c in excluded.iter().take(top_n) {
            let _ = writeln!(s, "- `{}`", c.id);
        }
        s.push('\n');
    }

    s.push_str("---\n\n");
    s.push_str(
        "본 issue는 6시간마다 자동 갱신돼요 (`update_existing: true`). 닫지 말고 검토 댓글로 처리해 주세요.\n",
    );

    s
}

fn write_candidate_block(s: &mut String, c: &Candidate, rank: usize) {
    let size = c
        .size_b
        .map(|b| format!("{}B", b))
        .unwrap_or_else(|| "?".to_string());
    let _ = writeln!(s, "### {}. `{}` — score {:.3}\n", rank, c.id, c.score);
    let _ = writeln!(
        s,
        "- size: {} / Open_LLM={:.2} / downloads={:.2} / korean={:.2} / license={:.2} / gguf={:.0}",
        size,
        c.components.open_llm,
        c.components.downloads,
        c.components.korean,
        c.components.license,
        c.components.gguf,
    );
    let _ = writeln!(s, "- HuggingFace: https://huggingface.co/{}\n", c.id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::ScoreComponents;

    fn make_candidate(id: &str, score: f64, queue: Queue, size_b: Option<f64>) -> Candidate {
        Candidate {
            id: id.into(),
            score,
            license_score: if queue == Queue::Excluded { 0.0 } else { 1.0 },
            size_b,
            queue,
            components: ScoreComponents {
                open_llm: 0.5,
                downloads: 0.5,
                korean: 0.5,
                license: 1.0,
                gguf: 1.0,
            },
        }
    }

    #[test]
    fn report_includes_frontmatter() {
        let cands = vec![make_candidate("x/y-7B", 0.5, Queue::Review, Some(7.0))];
        let r = generate_report(&cands, 10);
        // frontmatter 첫 줄.
        assert!(r.starts_with("---\n"));
        assert!(r.contains("title:"));
        assert!(r.contains("labels: auto-curate"));
        assert!(r.contains("assignees: freessunky-bit"));
        // 한국어 안내 포함.
        assert!(r.contains("큐레이터"));
    }

    #[test]
    fn report_groups_by_queue() {
        let cands = vec![
            make_candidate("a/b-7B", 0.7, Queue::Review, Some(7.0)),
            make_candidate("c/d-2B", 0.3, Queue::InfoOnly, Some(2.0)),
            make_candidate("e/f-5B", 0.0, Queue::Excluded, Some(5.0)),
        ];
        let r = generate_report(&cands, 10);
        assert!(r.contains("## Review queue"));
        assert!(r.contains("## Info-only"));
        assert!(r.contains("## Excluded"));
        assert!(r.contains("a/b-7B"));
        assert!(r.contains("c/d-2B"));
        assert!(r.contains("e/f-5B"));
    }

    #[test]
    fn report_sorts_by_score_desc() {
        let cands = vec![
            make_candidate("low/m-7B", 0.3, Queue::Review, Some(7.0)),
            make_candidate("hi/m-7B", 0.8, Queue::Review, Some(7.0)),
            make_candidate("mid/m-7B", 0.5, Queue::Review, Some(7.0)),
        ];
        let r = generate_report(&cands, 10);
        let hi_pos = r.find("hi/m-7B").unwrap();
        let mid_pos = r.find("mid/m-7B").unwrap();
        let low_pos = r.find("low/m-7B").unwrap();
        assert!(hi_pos < mid_pos, "hi(0.8) before mid(0.5)");
        assert!(mid_pos < low_pos, "mid(0.5) before low(0.3)");
    }

    #[test]
    fn report_top_n_limits_each_group() {
        let cands: Vec<Candidate> = (0..5)
            .map(|i| {
                make_candidate(
                    &format!("x/m{}-7B", i),
                    0.5 - (i as f64) * 0.05,
                    Queue::Review,
                    Some(7.0),
                )
            })
            .collect();
        let r = generate_report(&cands, 2);
        // top 2만 포함, m2~m4는 제외.
        assert!(r.contains("x/m0-7B"));
        assert!(r.contains("x/m1-7B"));
        assert!(!r.contains("x/m3-7B"));
        assert!(!r.contains("x/m4-7B"));
        // 단, 그룹 헤더 카운트는 *전체* (5건) 그대로 표시.
        assert!(r.contains("Review queue — 5 건"));
    }

    #[test]
    fn report_handles_empty() {
        let r = generate_report(&[], 10);
        assert!(r.contains("review queue 후보가 없어요") || r.contains("없어요"));
        // 빈 그룹도 헤더는 포함.
        assert!(r.contains("Review queue"));
    }
}
