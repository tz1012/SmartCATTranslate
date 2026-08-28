# SmartCAT Translate Implementation Roadmap

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 승인된 SmartCAT Translate 설계를 다섯 개의 독립적으로 검증 가능한 구현 계획으로 나누어 Windows와 macOS용 앱을 완성한다.

**Architecture:** React/TypeScript가 화면과 사용자 상태를 담당하고 Tauri 명령을 통해 Rust 서비스에 접근한다. Rust 서비스는 Codex App Server, 단축키, OCR, 문서 처리, 암호화 기록을 명확한 trait 경계 뒤에 배치하며 운영체제별 코드는 `platform` 모듈로 제한한다.

**Tech Stack:** Tauri 2, React, TypeScript, Rust, Vitest, React Testing Library, Cargo test, Playwright, GitHub Actions

**Spec:** `docs/superpowers/specs/2026-08-28-smartcat-translate-design.md`

## Global Constraints

- Windows와 macOS를 모두 지원한다.
- 데스크톱 셸은 Tauri 2, 화면은 React + TypeScript, 핵심 처리는 Rust를 사용한다.
- 번역 인증과 실행은 공식 Codex App Server를 사용한다.
- 원본 파일을 덮어쓰지 않고 새 번역 파일을 만든다.
- 번역 원문, 번역문, OCR 개인정보, 인증 토큰과 전체 로컬 경로를 개발 및 오류 로그에 남기지 않는다.
- 번역 세션에서 셸 명령과 임의 도구 실행을 허용하지 않는다.
- 모든 기능은 실패 테스트 작성, 실패 확인, 최소 구현, 통과 확인, 기록 갱신, 커밋 순서로 개발한다.
- `PROJECT_LOG.txt`, `DECISIONS.txt`, `CHANGELOG.md` 갱신 누락은 배포 검사 실패 사유다.

---

## Plan Order

1. `2026-08-28-foundation-codex-text.md`
   - 저장소 구조, 자동 기록 검사, 공통 계약, Codex 런타임과 인증, 텍스트 번역, 전체 창
   - 완료 결과: 사용자가 ChatGPT로 로그인하고 입력 텍스트를 번역할 수 있는 앱

2. `2026-08-28-hotkeys-popup.md`
   - 전역 단축키, 연속 입력, 충돌 분석, 클립보드 복원, 앱별 차단, 작은 팝업
   - 완료 결과: 다른 앱에서 선택한 텍스트를 단축키로 번역하고 팝업에서 복사할 수 있음

3. `2026-08-28-capture-image-translation.md`
   - 화면 영역 선택, 이미지 입력, Windows/macOS OCR, 좌표 정규화, 번역문 재배치
   - 완료 결과: 번역 이미지와 원문/번역문 비교 결과를 생성하고 저장할 수 있음

4. `2026-08-28-document-translation.md`
   - 공통 문서 작업, DOCX, PPTX, XLSX, PDF, 결과 보고서와 구조 검증
   - 완료 결과: 네 형식을 새 파일로 번역하고 손상 및 배치 변화를 보고할 수 있음

5. `2026-08-28-history-recovery-release.md`
   - 암호화 기록, 시크릿 모드, 작업 체크포인트, 재개, 업데이트, 서명 빌드와 배포 관문
   - 완료 결과: 복구와 개인정보 보호를 갖춘 Windows/macOS 사전 공개판

## Dependency Map

```text
foundation-codex-text
├── hotkeys-popup
├── capture-image-translation
└── document-translation
    └── history-recovery-release

hotkeys-popup ───────────┐
capture-image-translation ├─> history-recovery-release
document-translation ────┘
```

## Locked File Structure

```text
src/
  app/
    App.tsx
    routes.tsx
  features/
    account/
    translation/
    hotkeys/
    capture/
    documents/
    history/
    settings/
  lib/
    tauri.ts
    types.ts
  main.tsx

src-tauri/
  capabilities/
    default.json
  src/
    app_state.rs
    commands/
    core/
      audit.rs
      errors.rs
      types.rs
    codex/
      runtime.rs
      protocol.rs
      transport.rs
      auth.rs
      translation.rs
    hotkeys/
    capture/
    documents/
    storage/
    platform/
      windows/
      macos/
    lib.rs
    main.rs
  tests/
  Cargo.toml
  tauri.conf.json

scripts/
  record-change.mjs
  check-records.mjs

tests/
  fixtures/
  e2e/

.github/workflows/
```

각 파일은 한 책임만 갖는다. React 기능 폴더는 해당 화면, 상태와 테스트를 함께 두고 Rust 모듈은 외부 시스템별 trait와 구현을 함께 둔다. 운영체제 API 호출은 `platform/windows`와 `platform/macos` 밖으로 새지 않게 한다.

## Spec Coverage Matrix

| 설계 명세 절 | 구현 계획과 작업 |
|---|---|
| 1–2 목적과 범위 | 전체 로드맵, 다섯 계획의 완료 결과 |
| 3 기술 기반 | Foundation Task 1–5, Hotkeys Task 4, Capture Task 4 |
| 4 구성 요소 | Foundation Task 1, 3–9; 각 기능 계획의 명령/UI 작업 |
| 5 Codex 인증·보안 | Foundation Task 4–7 |
| 6 단축키 | Hotkeys Task 1–8 |
| 7 번역 설정 | Foundation Task 3, 7–9 |
| 8 이미지·화면 캡처 | Capture Task 1–7 |
| 9 문서 번역 | Documents Task 1–8 |
| 10 오류·복구 | Foundation Task 5–9, Recovery Task 3–4 |
| 11 개인정보 | Recovery Task 1–4 |
| 12 개발 기록 | Foundation Task 2, 모든 계획의 기록/커밋 단계 |
| 13 테스트 | 각 작업의 실패/통과 단계, Recovery Task 7 |
| 14 배포·업데이트 | Recovery Task 5–7 |
| 15 구현 단계 | 로드맵의 다섯 계획 순서 |
| 16 알려진 제약 | Hotkeys Task 3–4, Capture Task 4–6, Documents Task 4–7, Recovery Task 6 |
| 17 완료 기준 | 각 계획 마지막 acceptance task, Recovery Task 7 |

검토 결과 모든 설계 절이 하나 이상의 실패 테스트, 구현 작업과 검증 단계에 연결된다.
