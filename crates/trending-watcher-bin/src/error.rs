//! Trending Watcher 에러 — Phase 21'.b.
//!
//! 정책: 한국어 해요체 사용자 향 메시지 (Display).

#![allow(dead_code)]

use thiserror::Error;

pub type WatcherResult<T> = Result<T, WatcherError>;

#[derive(Debug, Error)]
pub enum WatcherError {
    #[error("HuggingFace API에 접속하지 못했어요: {0}")]
    HfApiUnreachable(String),

    #[error("HuggingFace rate limit 초과 — {retry_after_secs}초 후 다시 시도할게요")]
    RateLimited { retry_after_secs: u64 },

    #[error("응답 schema가 예상과 달라요: {0}")]
    SchemaMismatch(String),

    #[error("Parquet 파일을 읽지 못했어요: {0}")]
    ParquetReadFailed(String),

    #[error("내부 에러: {0}")]
    Internal(String),
}

impl From<reqwest::Error> for WatcherError {
    fn from(err: reqwest::Error) -> Self {
        Self::HfApiUnreachable(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_messages_korean() {
        assert!(WatcherError::HfApiUnreachable("x".into())
            .to_string()
            .contains("HuggingFace"));
        assert!(WatcherError::RateLimited {
            retry_after_secs: 60
        }
        .to_string()
        .contains("60초"));
        assert!(WatcherError::SchemaMismatch("x".into())
            .to_string()
            .contains("schema"));
    }
}
