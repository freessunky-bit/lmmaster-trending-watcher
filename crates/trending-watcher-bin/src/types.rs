//! Trending Watcher 도메인 타입 — Phase 21'.b.
//!
//! HuggingFace API + Open LLM Leaderboard 응답 schema의 *우리가 사용하는 필드만* 보존.
//! 전체 schema는 풀 deserialize하지 않아 schema drift 영향 최소화.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};

/// HF `/api/models?sort=trending` 응답의 한 row.
///
/// 응답 schema는 풍부하지만 본 watcher는 *필요한 필드만* 추출 — 다른 필드는 무시.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct TrendingModelMeta {
    /// `<author>/<repo>` (예: "Qwen/Qwen2.5-7B-Instruct").
    pub id: String,
    /// 다운로드 수 (HF는 fluid — 30일 누적 추정).
    #[serde(default)]
    pub downloads: u64,
    /// likes (좋아요).
    #[serde(default)]
    pub likes: u64,
    /// transformer / gguf / sentence-transformers / etc.
    #[serde(default, rename = "library_name")]
    pub library_name: Option<String>,
    /// tags — `["text-generation", "ko", "license:apache-2.0", ...]`.
    #[serde(default)]
    pub tags: Vec<String>,
    /// `text-generation` / `embedding` / `image-text-to-text` / etc.
    #[serde(default, rename = "pipeline_tag")]
    pub pipeline_tag: Option<String>,
    /// "2025-..." 등 RFC3339-ish.
    #[serde(default, rename = "createdAt")]
    pub created_at: Option<String>,
}

impl TrendingModelMeta {
    /// `tags`에서 `license:*` 추출. 없으면 `None`.
    pub fn license(&self) -> Option<String> {
        self.tags
            .iter()
            .find_map(|t| t.strip_prefix("license:").map(String::from))
    }

    /// `tags`에 `ko`가 있는지 (HF cardData.language=ko 자동 매핑).
    pub fn has_korean_tag(&self) -> bool {
        self.tags.iter().any(|t| t == "ko")
    }

    /// `library_name == "gguf"`이면 GGUF 라이브러리 모델.
    pub fn is_gguf(&self) -> bool {
        self.library_name.as_deref() == Some("gguf")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_minimal_payload() {
        let json = r#"{
            "id": "Qwen/Qwen2.5-7B-Instruct",
            "downloads": 1234567,
            "likes": 100,
            "library_name": "transformers",
            "tags": ["text-generation", "ko", "license:apache-2.0"],
            "pipeline_tag": "text-generation",
            "createdAt": "2025-09-01T00:00:00.000Z"
        }"#;
        let m: TrendingModelMeta = serde_json::from_str(json).unwrap();
        assert_eq!(m.id, "Qwen/Qwen2.5-7B-Instruct");
        assert_eq!(m.downloads, 1234567);
        assert_eq!(m.license(), Some("apache-2.0".into()));
        assert!(m.has_korean_tag());
        assert!(!m.is_gguf());
    }

    #[test]
    fn deserialize_unknown_fields_ignored() {
        // schema drift — 새 필드는 무시.
        let json = r#"{
            "id": "x/y",
            "downloads": 0,
            "ridiculous_new_field": {"nested": [1,2,3]}
        }"#;
        let m: TrendingModelMeta = serde_json::from_str(json).unwrap();
        assert_eq!(m.id, "x/y");
        assert!(m.tags.is_empty());
        assert!(m.license().is_none());
    }

    #[test]
    fn gguf_detection() {
        let mut m = TrendingModelMeta {
            id: "x/y".into(),
            downloads: 0,
            likes: 0,
            library_name: Some("gguf".into()),
            tags: Vec::new(),
            pipeline_tag: None,
            created_at: None,
        };
        assert!(m.is_gguf());
        m.library_name = Some("transformers".into());
        assert!(!m.is_gguf());
        m.library_name = None;
        assert!(!m.is_gguf());
    }
}
