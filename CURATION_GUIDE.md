# 큐레이션 가이드 — Trending Watcher 흐름

> 본 watcher가 만드는 *GitHub Issue*는 *큐레이터(사람)*가 검토해서 LMmaster 본 repo에 manifest PR을 올리는 데 사용해요. 자동 PR은 정책상 거부 (큐레이션 thesis 보존).

## 흐름

1. **Watcher가 6시간마다 실행** (GHA cron `0 */6 * * *`).
2. **HF Trending + Open LLM Leaderboard + Arena + KMMLU 규식 + Ollama 미러** fetch.
3. **Deterministic score 계산** (ADR-0059 §4 가중치 매트릭스).
4. **임계 통과** (license 화이트리스트 ✓ / 사이즈 3B~14B / GGUF 미러 또는 GGUF 라이브러리) **& dedupe**.
5. **GitHub Issue 생성** (JasonEtco/create-an-issue) — 본 repo 안. 라벨 `auto-curate` + `trending-watcher` + `needs-review`. 큐레이터 1인 assignee.
6. **큐레이터 검토** — 다음 5분 안에 결정 가능한 신호:
   - chat template 깨짐 여부 (HF 모델 카드 + tokenizer_config.json `chat_template` 필드)
   - 라이선스 함정 (whitelist 통과해도 *commercial use* 명시 필요)
   - 한국어 자연스러움 (모델 카드 한국어 예시 + KMMLU score 4 자릿수)
7. **양호 → LMmaster 본 repo에 manifest PR**:
   ```bash
   # LMmaster 본 repo에서:
   pnpm dlx @lmmaster/curate add <hub_id>
   # 또는 수동으로 manifests/snapshot/models/<category>/<id>.json 작성 +
   # node .claude/scripts/build-catalog-bundle.mjs로 번들 갱신.
   ```
8. **본 repo Issue 닫기** — manifest PR 머지 후.
9. **거부 → Issue에 코멘트 + close**. 사유는 *기각안 negative space* 차원에서 보존.

## 검토 체크리스트

- [ ] HF 모델 카드의 chat_template이 정상 로드되나?
- [ ] License가 *commercial use* 가능 여부 명시?
- [ ] 한국어 응답 자연스러운지 — 한국어 prompt 3건 즉석 검증 (LMmaster Workbench).
- [ ] GGUF 미러 (unsloth / bartowski / lmstudio-community / TheBloke / MaziyarPanahi) 실 동작?
- [ ] 사이즈가 3B~14B 정식 큐 또는 외이 info-only 큐?
- [ ] 기존 카탈로그 모델과 *중복 가치* 분석 (예: 동일 베이스 동일 사이즈면 우선순위 낮음).

## 라벨 정책

| 라벨 | 의미 |
|---|---|
| `auto-curate` | watcher가 만든 Issue |
| `trending-watcher` | watcher 출처 (다른 자동 도구와 구분용) |
| `needs-review` | 큐레이터 검토 대기 |
| `info-only` | 사이즈 게이트 외이 (3B 미만 또는 14B 초과) — 정식 큐 진입 X |
| `accepted` | 본 repo에 manifest PR 머지됨 |
| `rejected` | 검토 결과 거부 (사유 코멘트 필수) |

## 1주 운영 모니터링

Phase 21'.e DoD에 정의 — 첫 1주 동안 *false positive 비율 + dedupe 누락 / 중복 / Korean signal 임계 튜닝* 기록 후 ADR-0059 갱신.

## 외부 통신 정책

`huggingface.co`, `github.com`, `raw.githubusercontent.com`만 사용해요. 다른 도메인 추가는 ADR-0026 갱신 + 사용자 결정 필요.
