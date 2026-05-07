//! Open LLM Leaderboard 2 fetcher — Phase 21'.b.2 (ADR-0059 §3).
//!
//! 정책:
//! - dataset: `open-llm-leaderboard/contents` (configurable for fallback).
//! - HF API `/api/datasets/{repo}/parquet/{config}/{split}` → nested JSON `{config: {split: [URLs]}}`.
//! - 첫 shard parquet 전체 다운로드 (보통 수백 KB ~ 수 MB) → ArrowReader로 동기 파싱.
//! - 컬럼: `eval_name` (필수, Utf8) + `Average` (필수, Float64) + IFEval/BBH/MATH/GPQA/MUSR/MMLU-PRO (Optional).
//! - 모르는 컬럼은 무시 (schema drift 영향 최소화).

#![allow(dead_code)]

use std::collections::BTreeMap;

use arrow_array::{Array, Float64Array, RecordBatch, StringArray};
use bytes::Bytes;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use serde::Deserialize;

use crate::error::{WatcherError, WatcherResult};
use crate::types::LeaderboardEntry;

pub const HF_PARQUET_ENDPOINT: &str = "https://huggingface.co/api/datasets";
pub const OPEN_LLM_DATASET: &str = "open-llm-leaderboard/contents";

const USER_AGENT: &str = concat!("lmmaster-trending-watcher/", env!("CARGO_PKG_VERSION"));

/// HF parquet endpoint 응답 schema — `{config: {split: [URL...]}}` nested 또는 flat 배열.
#[derive(Debug, Deserialize)]
struct NestedParquetIndex {
    #[serde(flatten)]
    configs: BTreeMap<String, BTreeMap<String, Vec<String>>>,
}

/// 통합 — Open LLM Leaderboard 2 fetch + 파싱.
///
/// 정책:
/// - URL list 비면 graceful Ok(empty).
/// - 첫 shard만 사용 (Open LLM Leaderboard contents는 단일 shard).
/// - 추후 multi-shard 시 union (.b.3 후속).
pub async fn fetch_leaderboard(client: &reqwest::Client) -> WatcherResult<Vec<LeaderboardEntry>> {
    fetch_leaderboard_with_base(
        client,
        HF_PARQUET_ENDPOINT,
        OPEN_LLM_DATASET,
        "default",
        "train",
    )
    .await
}

/// 테스트용 — base URL + dataset + config + split 주입.
pub async fn fetch_leaderboard_with_base(
    client: &reqwest::Client,
    base: &str,
    dataset: &str,
    config: &str,
    split: &str,
) -> WatcherResult<Vec<LeaderboardEntry>> {
    let urls = resolve_parquet_urls(client, base, dataset, config, split).await?;
    if urls.is_empty() {
        tracing::warn!("Open LLM Leaderboard URL list가 비었어요 — graceful skip");
        return Ok(Vec::new());
    }
    let bytes = fetch_parquet_bytes(client, &urls[0]).await?;
    parse_leaderboard_parquet(&bytes)
}

/// HF API `/api/datasets/{ds}/parquet/{config}/{split}` 호출 + URL list 파싱.
pub async fn resolve_parquet_urls(
    client: &reqwest::Client,
    base: &str,
    dataset: &str,
    config: &str,
    split: &str,
) -> WatcherResult<Vec<String>> {
    let url = format!("{base}/{dataset}/parquet/{config}/{split}");
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
    parse_parquet_url_list(&body, config, split)
}

/// 응답 JSON parse — flat array 또는 nested 둘 다 시도.
pub fn parse_parquet_url_list(json: &str, config: &str, split: &str) -> WatcherResult<Vec<String>> {
    if let Ok(urls) = serde_json::from_str::<Vec<String>>(json) {
        return Ok(urls);
    }
    let idx: NestedParquetIndex = serde_json::from_str(json).map_err(|e| {
        WatcherError::SchemaMismatch(format!("parquet URL list schema 인식 실패: {e}"))
    })?;
    let urls = idx
        .configs
        .get(config)
        .and_then(|c| c.get(split))
        .cloned()
        .unwrap_or_default();
    Ok(urls)
}

/// Parquet bytes 전체 다운로드. Open LLM Leaderboard는 작은 파일이므로 Range 불필요.
pub async fn fetch_parquet_bytes(client: &reqwest::Client, url: &str) -> WatcherResult<Vec<u8>> {
    let resp = client
        .get(url)
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .send()
        .await
        .map_err(|e| WatcherError::HfApiUnreachable(format!("{url}: {e}")))?;
    if !resp.status().is_success() {
        return Err(WatcherError::HfApiUnreachable(format!(
            "HTTP {} for {url}",
            resp.status()
        )));
    }
    Ok(resp
        .bytes()
        .await
        .map_err(|e| WatcherError::HfApiUnreachable(format!("bytes: {e}")))?
        .to_vec())
}

/// Parquet bytes → Vec<LeaderboardEntry>.
///
/// 정책:
/// - 필수 컬럼 (`eval_name`, `Average`) 없으면 SchemaMismatch.
/// - Optional 컬럼은 dtype/null 둘 다 graceful (None).
/// - row 단위 — `eval_name` 또는 `Average` null이면 skip.
pub fn parse_leaderboard_parquet(bytes: &[u8]) -> WatcherResult<Vec<LeaderboardEntry>> {
    let buf = Bytes::copy_from_slice(bytes);
    let builder = ParquetRecordBatchReaderBuilder::try_new(buf)
        .map_err(|e| WatcherError::ParquetReadFailed(format!("builder: {e}")))?;
    let reader = builder
        .build()
        .map_err(|e| WatcherError::ParquetReadFailed(format!("reader: {e}")))?;

    let mut out = Vec::new();
    for batch_res in reader {
        let batch =
            batch_res.map_err(|e| WatcherError::ParquetReadFailed(format!("batch: {e}")))?;
        extract_rows(&batch, &mut out)?;
    }
    Ok(out)
}

fn extract_rows(batch: &RecordBatch, out: &mut Vec<LeaderboardEntry>) -> WatcherResult<()> {
    let eval_name = batch
        .column_by_name("eval_name")
        .ok_or_else(|| WatcherError::SchemaMismatch("'eval_name' 컬럼 없음".into()))?
        .as_any()
        .downcast_ref::<StringArray>()
        .ok_or_else(|| {
            WatcherError::SchemaMismatch("'eval_name'이 Utf8 StringArray 아님".into())
        })?;
    let average = batch
        .column_by_name("Average")
        .ok_or_else(|| WatcherError::SchemaMismatch("'Average' 컬럼 없음".into()))?
        .as_any()
        .downcast_ref::<Float64Array>()
        .ok_or_else(|| WatcherError::SchemaMismatch("'Average'이 Float64 아님".into()))?;

    for row in 0..batch.num_rows() {
        if eval_name.is_null(row) || average.is_null(row) {
            continue;
        }
        out.push(LeaderboardEntry {
            eval_name: eval_name.value(row).to_string(),
            average: average.value(row),
            ifeval: optional_f64(batch, "IFEval", row),
            bbh: optional_f64(batch, "BBH", row),
            math_lvl_5: optional_f64(batch, "MATH Lvl 5", row),
            gpqa: optional_f64(batch, "GPQA", row),
            musr: optional_f64(batch, "MUSR", row),
            mmlu_pro: optional_f64(batch, "MMLU-PRO", row),
        });
    }
    Ok(())
}

fn optional_f64(batch: &RecordBatch, name: &str, row: usize) -> Option<f64> {
    let arr = batch.column_by_name(name)?;
    let f = arr.as_any().downcast_ref::<Float64Array>()?;
    if f.is_null(row) {
        None
    } else {
        Some(f.value(row))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow_array::ArrayRef;
    use arrow_schema::{DataType, Field, Schema};
    use parquet::arrow::ArrowWriter;
    use std::sync::Arc;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// in-memory parquet 생성 helper — 테스트용.
    fn make_test_parquet(
        names: Vec<&str>,
        averages: Vec<f64>,
        ifevals: Vec<Option<f64>>,
    ) -> Vec<u8> {
        let schema = Arc::new(Schema::new(vec![
            Field::new("eval_name", DataType::Utf8, false),
            Field::new("Average", DataType::Float64, false),
            Field::new("IFEval", DataType::Float64, true),
        ]));
        let names_arr: ArrayRef = Arc::new(StringArray::from(names));
        let avg_arr: ArrayRef = Arc::new(Float64Array::from(averages));
        let if_arr: ArrayRef = Arc::new(Float64Array::from(ifevals));
        let batch = RecordBatch::try_new(schema.clone(), vec![names_arr, avg_arr, if_arr])
            .expect("RecordBatch::try_new");

        let mut buf = Vec::new();
        let mut writer =
            ArrowWriter::try_new(&mut buf, schema, None).expect("ArrowWriter::try_new");
        writer.write(&batch).expect("writer.write");
        writer.close().expect("writer.close");
        buf
    }

    /// **invariant 1** — URL resolver: nested {config: {split: urls}} 파싱.
    #[test]
    fn parse_nested_url_list() {
        let json = r#"{
            "default": {
                "train": [
                    "https://huggingface.co/datasets/x/resolve/refs%2Fconvert%2Fparquet/y/0000.parquet"
                ]
            }
        }"#;
        let urls = parse_parquet_url_list(json, "default", "train").unwrap();
        assert_eq!(urls.len(), 1);
        assert!(urls[0].contains("0000.parquet"));
    }

    /// **invariant 2** — URL resolver: flat array 형태도 허용 (호환성).
    #[test]
    fn parse_flat_url_list() {
        let json = r#"["https://huggingface.co/x.parquet"]"#;
        let urls = parse_parquet_url_list(json, "default", "train").unwrap();
        assert_eq!(urls.len(), 1);
    }

    /// **invariant 3** — URL resolver: 알 수 없는 schema → SchemaMismatch.
    #[test]
    fn parse_unknown_schema_errors() {
        let err = parse_parquet_url_list(r#"{"unexpected": 42}"#, "default", "train")
            .err()
            .unwrap();
        // unknown 형식이지만 *flat 배열도 가능* — flat 시도 실패 + nested 시도. nested는 BTreeMap 일치
        // (configs={"unexpected": ...})하지만 inner type mismatch라 SchemaMismatch.
        assert!(matches!(err, WatcherError::SchemaMismatch(_)));
    }

    /// **invariant 4** — Parquet 파싱: 정상 schema → Vec<LeaderboardEntry>.
    #[test]
    fn parse_basic_leaderboard_parquet() {
        let bytes = make_test_parquet(
            vec!["A/x", "B/y", "C/z"],
            vec![70.5, 65.0, 55.5],
            vec![Some(80.0), None, Some(60.0)],
        );
        let entries = parse_leaderboard_parquet(&bytes).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].eval_name, "A/x");
        assert_eq!(entries[0].average, 70.5);
        assert_eq!(entries[0].ifeval, Some(80.0));
        assert_eq!(entries[1].ifeval, None, "null IFEval → None");
        assert_eq!(entries[2].ifeval, Some(60.0));
    }

    /// **invariant 5** — 필수 컬럼 누락: SchemaMismatch.
    #[test]
    fn parse_missing_required_column() {
        // eval_name 없는 schema.
        let schema = Arc::new(Schema::new(vec![Field::new(
            "Average",
            DataType::Float64,
            false,
        )]));
        let avg: ArrayRef = Arc::new(Float64Array::from(vec![50.0]));
        let batch = RecordBatch::try_new(schema.clone(), vec![avg]).unwrap();
        let mut buf = Vec::new();
        let mut writer = ArrowWriter::try_new(&mut buf, schema, None).unwrap();
        writer.write(&batch).unwrap();
        writer.close().unwrap();

        let err = parse_leaderboard_parquet(&buf).unwrap_err();
        assert!(matches!(err, WatcherError::SchemaMismatch(_)));
    }

    /// **invariant 6** — Garbage parquet bytes → ParquetReadFailed.
    #[test]
    fn parse_garbage_bytes_returns_parquet_error() {
        let err = parse_leaderboard_parquet(b"not parquet").unwrap_err();
        assert!(matches!(err, WatcherError::ParquetReadFailed(_)));
    }

    /// **invariant 7** — URL resolver 429: RateLimit-Retry-After honor.
    #[tokio::test]
    async fn url_resolver_429_returns_rate_limited() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/datasets/test/repo/parquet/default/train"))
            .respond_with(ResponseTemplate::new(429).insert_header("RateLimit-Retry-After", "90"))
            .mount(&server)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let base = format!("{}/api/datasets", server.uri());
        let err = resolve_parquet_urls(&client, &base, "test/repo", "default", "train")
            .await
            .unwrap_err();
        match err {
            WatcherError::RateLimited { retry_after_secs } => {
                assert_eq!(retry_after_secs, 90);
            }
            other => panic!("expected RateLimited, got {other:?}"),
        }
    }

    /// **invariant 8** — URL resolver 404: HfApiUnreachable.
    #[tokio::test]
    async fn url_resolver_404_returns_unreachable() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/datasets/missing/repo/parquet/default/train"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let base = format!("{}/api/datasets", server.uri());
        let err = resolve_parquet_urls(&client, &base, "missing/repo", "default", "train")
            .await
            .unwrap_err();
        assert!(matches!(err, WatcherError::HfApiUnreachable(_)));
    }
}
