# Task 7 보안 수정 3 보고서

## 결과

SmartCAT 번역용 Codex를 stock/system 실행 파일이 아닌 감사 가능한 OpenAI Codex 0.144.4 하류 빌드로 고정했다. 최종 모델 요청의 도구 선언은 소스 수준에서 항상 0개이고, 사용자·프로젝트 지시문 탐색은 비활성화된다. 앱은 설치된 sidecar의 해시와 provenance를 확인한 뒤 동일 App Server 세션의 patch attestation까지 통과해야 계정·번역 서비스를 설치한다.

## 공급망과 실행 경계

- 업스트림 태그: `rust-v0.144.4`
- 업스트림 커밋: `8c68d4c87dc54d38861f5114e920c3de2efa5876`
- 소스 보관 파일 SHA-256: `14c173d78f0c22da73e4ca1a205836b525e1dd9fe7db9b4ddea62214b2cc5009`
- 패치 버전: `smartcat-1`
- 패치 SHA-256: `277656cea5ca940c30cf692bff1bcbe398ca18a60ed57bcdc6f0a1a82388704a`
- 대상: Windows x86_64, macOS x86_64, macOS arm64
- 앱 선택: 빌드 생성 매니페스트가 지정한 내장 sidecar만 허용; 시스템 탐색·다운로드 폴백 없음
- 실행 증명: 같은 stdio 세션에서 정확한 커밋·패치 버전·`toolCount=0`·`instructionDiscovery=false` 확인

고정 입력은 재현 가능하지만 다른 호스트에서 결과 바이트가 동일하다고 주장하지 않는다. 명시적 릴리스 workflow가 checksum, provenance, SBOM과 설치기 산출물을 만들며 이 작업에서는 공개하지 않았다.

## TDD 증거

RED:

1. 실제 Windows acceptance 테스트가 모호한 `OsString` 변환 때문에 컴파일되지 않았다.
2. 테스트가 프로세스 `start()`만 성공하면 stock Codex를 수락해 attestation 경계를 실제로 실행하지 않는 문제를 재현했다.
3. corrected 실제 patched 테스트는 initialize 뒤의 benign lifecycle 알림을 attestation 응답으로 오인해 `HandshakeFailed`가 됐다.
4. 장시간 로컬 thin-LTO 빌드는 업스트림 시험을 모두 통과했지만 최종 링크 중 호스트 실행 세션이 외부 종료되어 산출물이 없었다.

GREEN:

1. 테스트는 `start()` 뒤 `initialize()`를 반드시 호출하고 같은 세션 attestation을 확인한다.
2. 정확한 disabled remote-control lifecycle 알림 한 종류만 2프레임 제한으로 허용한다. 요청 ID, 활성 상태, 다른 메서드와 구조 변화는 거부한다.
3. 실제 patched sidecar initialize+attestation 1/1, 실제 stock Codex 거부 1/1 통과.
4. 업스트림 최종 요청 tool-free 1/1, 지시문 탐색 차단 1/1, attestation+빈 instruction sources 1/1 통과.
5. 악성 원문 fake JSONL 경계 1/1 통과: 기반·포크에는 원문이 없고 임시 턴에서만 처리하며 도구 이벤트는 실패 폐쇄한다.
6. 커밋 전 자체 검토에서 항상 거짓인 조건 아래 남아 있던 이전 AppContainer·Seatbelt 구현을 제거하고, 보안 집중 시험 7/7과 두 실제 acceptance를 다시 통과했다.
7. 지원 종료된 GitHub macOS 13 Intel 러너를 잡는 워크플로 계약 시험이 9/10 RED가 됐고, macOS 15 Intel과 macOS 15 arm64로 고정한 뒤 10/10 GREEN이 됐다.

## 전체 검증

- Rust: 112/112 통과
- 실제 Windows patched acceptance: 1/1 통과
- 실제 Windows stock rejection: 1/1 통과
- 하류 런타임 공급망 스크립트: 10/10 통과
- 프런트엔드: 17/17 통과
- TypeScript/Vite 프로덕션 빌드: 통과
- rustfmt: 통과
- Clippy 전체 대상: 기존 인증 트레잇의 단위 오류 반환 린트 하나만 좁게 허용하고 그 밖의 경고는 오류로 처리해 통과
- `git diff --check`: 통과
- patch 파일 실제 SHA-256과 pin 일치: 통과
- 유료/모델 네트워크 호출: 없음
- `sources/`: 변경 없음

## 변경 파일

수정:

- `.gitignore`
- `CHANGELOG.md`
- `DECISIONS.txt`
- `PROJECT_LOG.txt`
- `package.json`
- `src-tauri/Cargo.toml`
- `src-tauri/src/codex/bootstrap.rs`
- `src-tauri/src/codex/manifest.rs`
- `src-tauri/src/codex/process.rs`
- `src-tauri/src/codex/runtime.rs`
- `src-tauri/src/lib.rs`
- `src-tauri/test-support/fake_codex_process.rs`
- `src-tauri/tests/codex_bootstrap.rs`
- `src-tauri/tests/translation_coordinator.rs`

추가:

- `.github/workflows/smartcat-runtime-release.yml`
- `docs/superpowers/plans/2026-08-29-task-7-security-fix-round-3.md`
- `runtime-patches/codex-0.144.4-smartcat/PATCH-NOTICE.txt`
- `runtime-patches/codex-0.144.4-smartcat/pin.json`
- `runtime-patches/codex-0.144.4-smartcat/smartcat.patch`
- `scripts/build-smartcat-codex-runtime.mjs`
- `scripts/build-smartcat-codex-runtime.test.mjs`
- `src-tauri/tauri.runtime.conf.json`
- `task-7-report.md`

## 자체 검토와 열린 우려

- 앱의 제품 경로는 stock/system 런타임을 탐색하거나 실행하지 않는다. fake resolver의 system 후보는 테스트 전용이다.
- 패치가 upstream tool 모듈을 삭제하지는 않지만 최종 Responses 요청 조립 경계가 모든 provider·feature 상태에서 빈 도구 목록을 강제하며 upstream 시험이 이를 직접 검증한다.
- pinned 0.144.4의 정상 lifecycle 알림은 exact 구조와 disabled 상태만 허용한다. payload를 기록하거나 화면에 전달하지 않는다.
- 이전 AppContainer·Seatbelt 제품 구현과 관련 불필요 Windows 의존 기능은 제거되어 비활성 우회 코드가 남아 있지 않다.
- 로컬 제품용 thin-LTO sidecar와 생성 매니페스트/SBOM/provenance는 아직 없다. 동일 patched non-LTO sidecar로 실제 앱 경계를 검증했으며, 제품 산출물은 릴리스 CI가 성공하기 전에는 배포할 수 없다.
- 새 Rust 1.98의 엄격 Clippy 경고 1개는 기존 인증 트레잇에 있으며 이번 변경 범위를 넓혀 API를 바꾸지 않았다. 로컬과 릴리스 CI 모두 이 린트 하나만 좁게 허용하고 나머지 경고는 오류로 처리한다.
