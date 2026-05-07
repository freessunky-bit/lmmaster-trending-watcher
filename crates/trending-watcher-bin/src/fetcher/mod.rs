//! Trending Watcher fetcher 모듈 — Phase 21'.b.
//!
//! 외부 통신 화이트리스트 (ADR-0059 §2):
//! - `huggingface.co` — `/api/models?sort=trending`, `/api/datasets/{ds}/parquet/...`.
//! - `github.com` / `raw.githubusercontent.com` — Arena 미러, Ollama library.

pub mod hf_trending;

pub use hf_trending::fetch_hf_trending;
