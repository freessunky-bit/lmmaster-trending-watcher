# lmmaster-trending-watcher

> LMmaster 카탈로그 자동 갱신 watcher — 새 모델을 발견해 큐레이터에게 알려요.

[English](#english) | [한국어](#한국어)

---

## 한국어

### 무엇이에요?

[LMmaster](https://github.com/freessunky-bit/lmmaster) (Korean-first AI desktop companion)의 카탈로그를 *자동으로 최신 상태로 유지*해 주는 별도 도구입니다. 6시간마다 HuggingFace + Open LLM Leaderboard + Arena를 점검해서 *조건을 만족하는 새 모델*을 발견하면 GitHub Issue로 큐레이터에게 알려 드려요.

### 왜 별도 repo 인가요?

- **사용자 PC와 무관** — GitHub Actions 위에서만 돌아가요. 사용자 데스크톱에 설치 부담 0.
- **public audit-able** — score 계산식 + 가중치 + 제외 사유가 모두 공개돼 있어요.
- **secrets 관리 분리** — LMmaster 본 repo와 권한/토큰이 섞이지 않아요.
- **자동 PR 거부** — Issue로만 알려요. 큐레이터(=사람)가 직접 검토해서 본 repo에 manifest PR을 올려야 카탈로그에 합류해요. 큐레이션 thesis 보존.

### 데이터 소스 (ADR-0059 §3)

| 소스 | 용도 |
|---|---|
| HuggingFace `/api/models?sort=trending` | 발견 1차 |
| Open LLM Leaderboard 2 (Parquet) | 벤치 점수 + chat template + license ground truth |
| Arena 미러 (oolong-tea-2026/arena-ai-leaderboards) | LMSYS ELO 보조 |
| 모델 카드 KMMLU 정규식 | 한국어 1차 검증 |
| Ollama library 미러 | GGUF 미러 존재 binary signal |

### Deterministic 점수 (LLM judge 0)

```
score = 0.35·norm(Open_LLM_Avg)
      + 0.20·log10(downloads_30d)
      + 0.20·korean_signal
      + 0.15·license_score
      + 0.10·gguf_present
```

- `license_score`: apache-2/mit = 1.0, llama3.x-community/gemma = 0.7, exaone/nvidia-open = 0.4, 그 외 = 0.0 (자동 제외).
- `korean_signal`: `language=ko` 1.0 / 본문 한국어 키워드 hit 0.3·count cap 1.0 / 미언급 0.0.
- 사이즈 게이트: 3B~14B만 정식 큐. 외이는 info-only.

### 흐름

```
6h cron → fetch 4종 → score → license 화이트리스트 → 사이즈 게이트 → 후보 추리기
                                                                            ↓
              JasonEtco/create-an-issue (dedupe 라벨 `auto-curate`)
                                                                            ↓
                                                  큐레이터 1인 검토
                                                                            ↓
                              LMmaster 본 repo에 manifest PR (수동 작성)
```

### 외부 통신 화이트리스트

`huggingface.co`, `github.com`, `raw.githubusercontent.com`만 사용해요. 다른 도메인 추가는 별도 ADR + 사용자 결정이 필요해요.

### 라이선스

MIT.

### 운영 관련 문의

LMmaster 본 repo의 [issue](https://github.com/freessunky-bit/lmmaster/issues)로 부탁드려요.

---

## English

### What is this?

A separate watcher that keeps the [LMmaster](https://github.com/freessunky-bit/lmmaster) (Korean-first AI desktop companion) catalog *automatically up to date*. Every 6 hours it scans HuggingFace + Open LLM Leaderboard + Arena and *files a GitHub Issue* for the curator when a model meets the criteria.

### Why a separate repo?

- **No user PC dependency** — runs only on GitHub Actions. Zero install burden.
- **Public auditable** — scoring formula + weights + exclusion rules are open.
- **Secrets isolation** — separate from the main LMmaster repo permissions.
- **No auto-PR** — only Issues. A human curator manually opens manifest PRs after review. Preserves the curation thesis.

### Data sources (ADR-0059 §3)

| Source | Purpose |
|---|---|
| HuggingFace `/api/models?sort=trending` | Primary discovery |
| Open LLM Leaderboard 2 (Parquet) | Benchmark + chat template + license ground truth |
| Arena mirror (oolong-tea-2026/arena-ai-leaderboards) | LMSYS ELO secondary |
| Model card KMMLU regex | Korean primary verification |
| Ollama library mirror | GGUF mirror binary signal |

### Deterministic scoring (zero LLM judge)

Same formula as Korean section above.

### License

MIT.
