//! Deterministic 필터 매트릭스 — Phase 21'.c (ADR-0059 §4).
//!
//! 정책 (ADR-0059):
//! ```text
//! score = 0.35 · norm(Open_LLM_Avg)    // 0..100 → 0..1
//!       + 0.20 · log10(1 + downloads_30d)  / 9.0  // ~10^0..10^9 → 0..1
//!       + 0.20 · korean_signal           // 0..1
//!       + 0.15 · license_score           // 0..1
//!       + 0.10 · gguf_present            // 0 or 1
//! ```
//!
//! - License whitelist:
//!   - `apache-2.0` / `mit`           → 1.0
//!   - `llama3.x-community` / `gemma` → 0.7
//!   - `exaone` / `nvidia-open`       → 0.4
//!   - 그 외                           → 0.0 (자동 제외)
//! - 사이즈 게이트: 3B ~ 14B만 정식 큐. 외이는 info-only.
//! - Korean signal: `tags.contains("ko")` 1.0, 본문 정규식 hit (.c.2 후속).
//! - LLM judge 0 — score 100% 코드 상수.
//!
//! deterministic 정합성 — 동일 input 100회 호출 동일 output.

#![allow(dead_code)]

use crate::types::{LeaderboardEntry, TrendingModelMeta};

/// 가중치 상수 — ADR-0059 §4. 합 1.0.
pub const W_OPEN_LLM: f64 = 0.35;
pub const W_DOWNLOADS: f64 = 0.20;
pub const W_KOREAN: f64 = 0.20;
pub const W_LICENSE: f64 = 0.15;
pub const W_GGUF: f64 = 0.10;

/// log10 정규화 — 10^9 다운로드를 1.0으로 cap.
const DOWNLOADS_LOG_CAP: f64 = 9.0;

/// 사이즈 게이트 — 3B ~ 14B 정식 큐.
pub const SIZE_GATE_MIN_B: f64 = 3.0;
pub const SIZE_GATE_MAX_B: f64 = 14.0;

/// Trending + Leaderboard + 메타를 합쳐 점수와 분류 결과를 반환.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub id: String,
    pub score: f64,
    pub license_score: f64,
    pub size_b: Option<f64>,
    pub queue: Queue,
    /// 디버깅용 — 점수 구성 component.
    pub components: ScoreComponents,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Queue {
    /// 3B~14B + license 통과 + score ≥ 임계 → 큐레이터 review queue.
    Review,
    /// 사이즈 외이 (< 3B 또는 > 14B) — info-only.
    InfoOnly,
    /// license 0.0 또는 score 0.0 — 자동 제외.
    Excluded,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScoreComponents {
    pub open_llm: f64,
    pub downloads: f64,
    pub korean: f64,
    pub license: f64,
    pub gguf: f64,
}

impl ScoreComponents {
    pub fn weighted_sum(self) -> f64 {
        W_OPEN_LLM * self.open_llm
            + W_DOWNLOADS * self.downloads
            + W_KOREAN * self.korean
            + W_LICENSE * self.license
            + W_GGUF * self.gguf
    }
}

/// `apache-2.0` / `mit` / `llama3.x-community` / `gemma` / `exaone` / `nvidia-open` whitelist.
/// 그 외는 0.0 (자동 제외).
pub fn license_score(license: &str) -> f64 {
    let l = license.to_ascii_lowercase();
    if l == "apache-2.0" || l == "mit" || l == "bsd-3-clause" {
        1.0
    } else if l.starts_with("llama3") || l == "gemma" || l == "gemma2" {
        0.7
    } else if l == "exaone"
        || l == "nvidia-open"
        || l == "nvidia-open-model-license"
        || l.starts_with("nvidia")
    {
        0.4
    } else {
        0.0
    }
}

/// downloads → 0..1. log10(1+x) / 9.0 cap.
pub fn downloads_score(downloads: u64) -> f64 {
    if downloads == 0 {
        return 0.0;
    }
    let log10 = ((downloads as f64) + 1.0).log10();
    (log10 / DOWNLOADS_LOG_CAP).clamp(0.0, 1.0)
}

/// Open LLM Leaderboard `Average` (0~100) → 0..1. 없으면 0.0.
pub fn open_llm_score(avg: Option<f64>) -> f64 {
    avg.map(|a| (a / 100.0).clamp(0.0, 1.0)).unwrap_or(0.0)
}

/// Korean signal — Phase 21'.c.1은 *tags["ko"]만 검사*. 정규식 hit는 .c.2 후속.
pub fn korean_signal(model: &TrendingModelMeta) -> f64 {
    if model.has_korean_tag() {
        1.0
    } else {
        0.0
    }
}

/// `library_name == "gguf"` 또는 `hub_id`가 GGUF 미러 author로 시작하는지.
pub fn gguf_present_score(model: &TrendingModelMeta) -> f64 {
    if model.is_gguf() {
        return 1.0;
    }
    let id_lower = model.id.to_ascii_lowercase();
    let mirrors = [
        "unsloth/",
        "bartowski/",
        "lmstudio-community/",
        "thebloke/",
        "maziyarpanahi/",
    ];
    if mirrors.iter().any(|m| id_lower.starts_with(m)) {
        1.0
    } else {
        0.0
    }
}

/// 모델 id에서 *parameter count (B)* 정규식 추출 — 예: "Qwen/Qwen2.5-7B" → 7.0.
///
/// 다중 매치 시 *마지막 hit*이 모델 사이즈 (예: "1.5B-Instruct" 보다는 "Qwen2-7B-1.5B" 같은 변종 케이스).
pub fn extract_param_count_b(id: &str) -> Option<f64> {
    use regex::Regex;
    use std::sync::OnceLock;
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        // `\b\d+(?:\.\d+)?[Bb]\b` — 단어 경계 + 숫자(소수 옵션) + B.
        Regex::new(r"(?i)\b(\d+(?:\.\d+)?)b\b").expect("param regex")
    });
    re.captures_iter(id)
        .filter_map(|c| c.get(1))
        .filter_map(|m| m.as_str().parse::<f64>().ok())
        .last()
}

/// 사이즈 게이트 — 3B ~ 14B 정식, 외이 info-only.
pub fn size_gate(size_b: Option<f64>) -> Queue {
    match size_b {
        Some(b) if (SIZE_GATE_MIN_B..=SIZE_GATE_MAX_B).contains(&b) => Queue::Review,
        Some(_) => Queue::InfoOnly, // 사이즈 검출됐으나 외이.
        None => Queue::InfoOnly,    // 사이즈 검출 X — 자동 제외 X (사람 검토).
    }
}

/// Trending model + Optional Leaderboard entry → Candidate.
///
/// 정책:
/// - license_score 0.0이면 즉시 `Excluded` (큐 진입 X).
/// - 사이즈 게이트로 `Review` / `InfoOnly` 분기.
/// - score는 components 가중합 (LLM judge 0).
pub fn score_candidate(
    model: &TrendingModelMeta,
    leaderboard: Option<&LeaderboardEntry>,
) -> Candidate {
    let lic_str = model.license().unwrap_or_default();
    let lic = license_score(&lic_str);
    let dl = downloads_score(model.downloads);
    let kr = korean_signal(model);
    let gg = gguf_present_score(model);
    let ol = open_llm_score(leaderboard.map(|e| e.average));

    let components = ScoreComponents {
        open_llm: ol,
        downloads: dl,
        korean: kr,
        license: lic,
        gguf: gg,
    };
    let score = components.weighted_sum();
    let size_b = extract_param_count_b(&model.id);
    let queue = if lic == 0.0 {
        Queue::Excluded
    } else {
        size_gate(size_b)
    };

    Candidate {
        id: model.id.clone(),
        score,
        license_score: lic,
        size_b,
        queue,
        components,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_model(
        id: &str,
        downloads: u64,
        tags: Vec<&str>,
        library: Option<&str>,
    ) -> TrendingModelMeta {
        TrendingModelMeta {
            id: id.to_string(),
            downloads,
            likes: 0,
            library_name: library.map(String::from),
            tags: tags.into_iter().map(String::from).collect(),
            pipeline_tag: None,
            created_at: None,
        }
    }

    /// **invariant 1** — 가중치 합 1.0.
    #[test]
    fn weights_sum_to_one() {
        let total = W_OPEN_LLM + W_DOWNLOADS + W_KOREAN + W_LICENSE + W_GGUF;
        assert!(
            (total - 1.0).abs() < f64::EPSILON,
            "weights must sum to 1.0"
        );
    }

    /// **invariant 2** — license whitelist 매핑.
    #[test]
    fn license_score_whitelist() {
        assert_eq!(license_score("apache-2.0"), 1.0);
        assert_eq!(license_score("mit"), 1.0);
        assert_eq!(license_score("MIT"), 1.0); // case-insensitive
        assert_eq!(license_score("llama3.1-community"), 0.7);
        assert_eq!(license_score("gemma"), 0.7);
        assert_eq!(license_score("exaone"), 0.4);
        assert_eq!(license_score("nvidia-open-model-license"), 0.4);
        assert_eq!(license_score("other"), 0.0);
        assert_eq!(license_score(""), 0.0);
    }

    /// **invariant 3** — size gate 분기.
    #[test]
    fn size_gate_buckets() {
        assert_eq!(size_gate(Some(3.0)), Queue::Review);
        assert_eq!(size_gate(Some(7.0)), Queue::Review);
        assert_eq!(size_gate(Some(14.0)), Queue::Review);
        assert_eq!(size_gate(Some(2.9)), Queue::InfoOnly);
        assert_eq!(size_gate(Some(15.0)), Queue::InfoOnly);
        assert_eq!(size_gate(Some(70.0)), Queue::InfoOnly);
        assert_eq!(size_gate(None), Queue::InfoOnly);
    }

    /// **invariant 4** — extract_param_count_b 정규식.
    #[test]
    fn extract_param_count_basic() {
        assert_eq!(extract_param_count_b("Qwen/Qwen2.5-7B-Instruct"), Some(7.0));
        assert_eq!(
            extract_param_count_b("meta-llama/Llama-3.1-8B-Instruct"),
            Some(8.0)
        );
        assert_eq!(
            extract_param_count_b("nvidia/Nemotron-3-Nano-4B"),
            Some(4.0)
        );
        assert_eq!(
            extract_param_count_b("Qwen/Qwen2.5-1.5B-Instruct"),
            Some(1.5)
        );
        assert_eq!(extract_param_count_b("ns/no-size-here"), None);
        // 주석에 명시: 다중 매치 시 마지막. (실제 use case는 single match가 일반.)
    }

    /// **invariant 5** — downloads_score: 0 → 0.0, 10^9 → 1.0 cap.
    #[test]
    fn downloads_score_curve() {
        assert_eq!(downloads_score(0), 0.0);
        assert!(downloads_score(1_000) > 0.0);
        assert!(downloads_score(1_000_000) > downloads_score(1_000));
        assert!(downloads_score(10_000_000_000).abs() <= 1.0 + f64::EPSILON);
    }

    /// **invariant 6** — open_llm_score: None → 0.0, 100 → 1.0.
    #[test]
    fn open_llm_score_curve() {
        assert_eq!(open_llm_score(None), 0.0);
        assert_eq!(open_llm_score(Some(0.0)), 0.0);
        assert_eq!(open_llm_score(Some(50.0)), 0.5);
        assert_eq!(open_llm_score(Some(100.0)), 1.0);
        assert_eq!(open_llm_score(Some(150.0)), 1.0); // clamp
    }

    /// **invariant 7** — score deterministic: 동일 input 100회 동일 output.
    #[test]
    fn score_deterministic_100x() {
        let model = make_model(
            "Qwen/Qwen2.5-7B-Instruct",
            1_000_000,
            vec!["text-generation", "ko", "license:apache-2.0"],
            Some("transformers"),
        );
        let lb = Some(LeaderboardEntry {
            eval_name: "Qwen/Qwen2.5-7B-Instruct".into(),
            average: 70.0,
            ifeval: None,
            bbh: None,
            math_lvl_5: None,
            gpqa: None,
            musr: None,
            mmlu_pro: None,
        });
        let first = score_candidate(&model, lb.as_ref());
        for _ in 0..100 {
            let next = score_candidate(&model, lb.as_ref());
            assert_eq!(first, next, "score must be deterministic");
        }
    }

    /// **invariant 8** — license 0.0이면 Excluded queue.
    #[test]
    fn license_zero_excludes_candidate() {
        let model = make_model(
            "x/y-7B",
            1_000,
            vec!["license:other"], // not in whitelist
            None,
        );
        let cand = score_candidate(&model, None);
        assert_eq!(cand.queue, Queue::Excluded);
        assert_eq!(cand.license_score, 0.0);
    }

    /// **invariant 9** — Korean signal: tags["ko"] → 1.0, 그 외 → 0.0.
    #[test]
    fn korean_signal_tag_only() {
        let m_ko = make_model("x/y-7B", 0, vec!["ko"], None);
        let m_en = make_model("x/y-7B", 0, vec!["en"], None);
        assert_eq!(korean_signal(&m_ko), 1.0);
        assert_eq!(korean_signal(&m_en), 0.0);
    }

    /// **invariant 10** — GGUF: library_name 또는 미러 author.
    #[test]
    fn gguf_detection_library_or_mirror() {
        // direct gguf library.
        let m1 = make_model("Qwen/Qwen2.5-GGUF", 0, vec![], Some("gguf"));
        assert_eq!(gguf_present_score(&m1), 1.0);
        // mirror author.
        let m2 = make_model(
            "bartowski/Qwen2.5-7B-Instruct-GGUF",
            0,
            vec![],
            Some("gguf"),
        );
        assert_eq!(gguf_present_score(&m2), 1.0);
        // unsloth lower-cased path also OK.
        let m3 = make_model("unsloth/llama-3.1-8b", 0, vec![], None);
        assert_eq!(gguf_present_score(&m3), 1.0);
        // neither.
        let m4 = make_model(
            "meta-llama/Llama-3.1-8B-Instruct",
            0,
            vec![],
            Some("transformers"),
        );
        assert_eq!(gguf_present_score(&m4), 0.0);
    }
}
