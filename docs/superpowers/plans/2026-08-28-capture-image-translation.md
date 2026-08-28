# Image and Screen Capture Translation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 이미지 파일과 사용자가 선택한 화면 영역에서 OCR 좌표를 추출하고 번역문을 원래 위치에 배치한 새 이미지와 비교 결과를 제공한다.

**Architecture:** 캡처, OCR, 번역, 배경 복원과 텍스트 배치를 각각 trait로 분리한다. Windows OCR과 macOS Vision은 동일한 `OcrEngine` 결과 형식을 반환하고 공통 Rust 파이프라인이 좌표 정규화, 블록 번역과 렌더링을 처리한다.

**Tech Stack:** Tauri 2, React, TypeScript, Rust, image, imageproc, cosmic-text, xcap, Windows.Media.Ocr, macOS Vision, Cargo test, Vitest, Playwright

**Spec:** `docs/superpowers/specs/2026-08-28-smartcat-translate-design.md`

## Global Constraints

- 이미지 입력과 화면 영역 캡처를 모두 지원한다.
- 번역 이미지를 저장하고 원문과 번역문을 각각 복사할 수 있다.
- OCR 좌표, 방향과 신뢰도를 보존한다.
- 낮은 OCR 신뢰도와 복잡한 배경을 사용자에게 표시한다.
- 원본 이미지는 변경하지 않는다.
- 캡처 권한이 없으면 운영체제별 설정 안내를 제공한다.

---

### Task 1: Geometry, OCR and render contracts

**Files:**
- Create: `src-tauri/src/capture/mod.rs`
- Create: `src-tauri/src/capture/types.rs`
- Create: `src/features/capture/types.ts`
- Test: `src-tauri/src/capture/types.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `PROJECT_LOG.txt`

**Interfaces:**
- Produces: `PixelRect`, `NormalizedRect`, `OcrLine`, `OcrDocument`, `TranslatedBlock`, `CaptureJobResult`

- [ ] **Step 1: Write failing geometry tests**

```rust
#[test]
fn normalizes_and_denormalizes_rectangles_without_drift() {
    let pixel = PixelRect { x: 120, y: 80, width: 400, height: 60 };
    let normalized = pixel.normalize(1920, 1080).unwrap();
    assert_eq!(normalized.denormalize(1920, 1080), pixel);
}

#[test]
fn rejects_rectangles_outside_the_image() {
    let rect = PixelRect { x: 1900, y: 10, width: 100, height: 20 };
    assert!(rect.validate(1920, 1080).is_err());
}
```

Run: `cargo test --manifest-path src-tauri/Cargo.toml capture::types`

Expected: FAIL because capture types do not exist.

- [ ] **Step 2: Define serializable contracts**

```rust
#[derive(Clone, Copy, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedRect { pub x: f32, pub y: f32, pub width: f32, pub height: f32 }

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrLine {
    pub id: uuid::Uuid,
    pub text: String,
    pub bounds: NormalizedRect,
    pub confidence: f32,
    pub angle_degrees: f32,
    pub direction: TextDirection,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslatedBlock {
    pub source_ids: Vec<uuid::Uuid>,
    pub source_text: String,
    pub translated_text: String,
    pub bounds: NormalizedRect,
    pub confidence: f32,
}
```

Mirror field names in TypeScript. Reject NaN, infinity, negative dimensions and normalized values outside 0..=1.

- [ ] **Step 3: Verify serialization parity and commit**

Serialize one Rust fixture to JSON and load the same fixture in a Vitest test to assert camelCase keys and enum strings.

Run: `cargo test --manifest-path src-tauri/Cargo.toml capture::types && pnpm test -- capture/types`

Expected: PASS.

```bash
git add src-tauri/src/capture src/features/capture src-tauri/src/lib.rs PROJECT_LOG.txt
git commit -m "feat: define OCR and image layout contracts"
```

### Task 2: Safe image import and immutable source handling

**Files:**
- Create: `src-tauri/src/capture/image_input.rs`
- Create: `src-tauri/tests/image_input.rs`
- Create: `tests/fixtures/images/simple-sign.png`
- Create: `tests/fixtures/images/rotated-label.jpg`
- Modify: `src-tauri/Cargo.toml`
- Modify: `PROJECT_LOG.txt`

**Interfaces:**
- Produces: `ImageInput::open_read_only`, `DecodedImage`, `SourceFingerprint`

- [ ] **Step 1: Write failing file-safety tests**

```rust
#[test]
fn opens_supported_images_without_changing_source() {
    let before = sha256(fixture("simple-sign.png"));
    let decoded = ImageInput::open_read_only(fixture("simple-sign.png")).unwrap();
    assert_eq!((decoded.width, decoded.height), (800, 450));
    assert_eq!(sha256(fixture("simple-sign.png")), before);
}

#[test]
fn rejects_decompression_bombs() {
    let error = ImageInput::from_header(100_000, 100_000).unwrap_err();
    assert!(matches!(error, ImageInputError::PixelLimitExceeded));
}
```

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test image_input`

Expected: FAIL because `ImageInput` does not exist.

- [ ] **Step 2: Implement bounded decoding**

Allow PNG, JPEG, WebP, TIFF and BMP. Enforce 80 million pixels, 200 MB decoded memory and 50 MB input by default. Apply EXIF orientation before OCR and retain the original hash and dimensions in `SourceFingerprint`. Read through `OpenOptions::new().read(true)` only.

- [ ] **Step 3: Verify fixtures and commit**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test image_input`

Expected: PASS for orientation, alpha, corrupt files, unsupported format, size limit and original hash.

```bash
git add src-tauri/src/capture/image_input.rs src-tauri/tests/image_input.rs tests/fixtures/images src-tauri/Cargo.toml PROJECT_LOG.txt
git commit -m "feat: import images without modifying originals"
```

### Task 3: Cross-platform screen region capture

**Files:**
- Create: `src/features/capture/CaptureOverlay.tsx`
- Create: `src/features/capture/CaptureOverlay.test.tsx`
- Create: `src-tauri/src/capture/screen.rs`
- Create: `src-tauri/src/commands/capture.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/tauri.conf.json`
- Modify: `src-tauri/Info.plist`
- Modify: `PROJECT_LOG.txt`

**Interfaces:**
- Produces: `ScreenCapturePort`, `CaptureSelection`, Tauri `start_screen_capture`, `complete_screen_capture`

- [ ] **Step 1: Write failing overlay tests**

Test drag in every direction, keyboard arrow resizing, Escape cancellation, Enter confirmation, minimum 8×8 selection, multiple monitors with negative coordinates and 125% scaling.

Run: `pnpm test -- CaptureOverlay.test.tsx`

Expected: FAIL because the overlay does not exist.

- [ ] **Step 2: Implement coordinate conversion and capture port**

```rust
#[async_trait::async_trait]
pub trait ScreenCapturePort: Send + Sync {
    async fn monitors(&self) -> Result<Vec<MonitorInfo>, CaptureError>;
    async fn capture(&self, selection: CaptureSelection) -> Result<DecodedImage, CaptureError>;
    async fn permission(&self) -> Result<CapturePermission, CaptureError>;
}
```

Use logical coordinates in React and convert once to physical pixels using the selected monitor scale factor. Use `xcap` for pixels. On macOS return `PermissionRequired { settings_url }` before opening the overlay when screen-recording permission is absent.

- [ ] **Step 3: Implement the transparent overlay flow**

Create one frameless transparent window per monitor, freeze monitor screenshots behind the selection layer, synchronize one global physical rectangle, and close every overlay on complete or cancel. The selection border must remain visible in light and dark backgrounds without appearing in the captured image.

Run: `pnpm test -- CaptureOverlay.test.tsx && cargo test --manifest-path src-tauri/Cargo.toml capture::screen`

Expected: PASS.

- [ ] **Step 4: Commit screen capture**

```bash
git add src/features/capture src-tauri/src/capture/screen.rs src-tauri/src/commands/capture.rs src-tauri/src/commands/mod.rs src-tauri/src/lib.rs src-tauri/tauri.conf.json src-tauri/Info.plist PROJECT_LOG.txt
git commit -m "feat: select and capture screen regions"
```

### Task 4: Windows and macOS OCR adapters

**Files:**
- Create: `src-tauri/src/capture/ocr.rs`
- Create: `src-tauri/src/platform/windows/ocr.rs`
- Create: `src-tauri/src/platform/macos/ocr.rs`
- Create: `src-tauri/tests/ocr_contract.rs`
- Modify: `src-tauri/src/platform/windows/mod.rs`
- Modify: `src-tauri/src/platform/macos/mod.rs`
- Modify: `src-tauri/Cargo.toml`
- Modify: `PROJECT_LOG.txt`

**Interfaces:**
- Consumes: `DecodedImage`
- Produces: `OcrEngine::recognize(&DecodedImage, &[String]) -> OcrDocument`

- [ ] **Step 1: Write failing adapter contract tests**

```rust
#[tokio::test]
async fn ocr_results_are_sorted_in_reading_order_and_normalized() {
    let result = fixture_engine().recognize(&fixture_image(), &["en".into(), "ko".into()]).await.unwrap();
    assert!(result.lines.windows(2).all(|pair| reading_order(pair[0].bounds, pair[1].bounds)));
    assert!(result.lines.iter().all(|line| line.bounds.is_normalized()));
    assert!(result.lines.iter().all(|line| (0.0..=1.0).contains(&line.confidence)));
}
```

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test ocr_contract`

Expected: FAIL because `OcrEngine` does not exist.

- [ ] **Step 2: Implement the common OCR normalization layer**

```rust
#[async_trait::async_trait]
pub trait OcrEngine: Send + Sync {
    async fn recognize(&self, image: &DecodedImage, language_hints: &[String])
        -> Result<OcrDocument, OcrError>;
}
```

Convert native pixel/bottom-left coordinates to normalized top-left coordinates, normalize Unicode to NFC, preserve line angle, clamp confidence and sort by writing direction and row.

- [ ] **Step 3: Implement Windows.Media.Ocr**

Create `SoftwareBitmap` from BGRA8 pixels, choose `OcrEngine::TryCreateFromLanguage` for the first installed hint and fall back to user profile languages. Map `OcrLine` and `OcrWord.BoundingRect` to the common format. Return `LanguagePackMissing` with the requested tags when no engine can be created.

- [ ] **Step 4: Implement macOS Vision OCR**

Create `VNRecognizeTextRequest` with accurate recognition, pass supported language hints, enable language correction, and map each top candidate and normalized Vision bounding box to the common top-left format. Return `UnsupportedOsVersion` below the declared minimum macOS version.

- [ ] **Step 5: Verify on both CI platforms and commit**

Run fixture contract tests on `windows-latest` and `macos-latest`; store only expected text and coordinates, not user screenshots.

```bash
git add src-tauri/src/capture/ocr.rs src-tauri/src/platform src-tauri/tests/ocr_contract.rs src-tauri/Cargo.toml PROJECT_LOG.txt
git commit -m "feat: recognize text on Windows and macOS"
```

### Task 5: OCR block grouping and structured translation

**Files:**
- Create: `src-tauri/src/capture/layout.rs`
- Create: `src-tauri/src/capture/translate.rs`
- Create: `src-tauri/tests/capture_translation.rs`
- Modify: `PROJECT_LOG.txt`

**Interfaces:**
- Consumes: `OcrDocument`, `TranslationBackend`, `TranslationProfile`
- Produces: `group_lines(&OcrDocument) -> Vec<TextBlock>`, `translate_blocks -> Vec<TranslatedBlock>`

- [ ] **Step 1: Write failing layout and token tests**

Test that nearby same-angle lines join, distant columns stay separate, table cells stay separate, low-confidence lines are marked, protected URLs survive exactly, and returned source IDs match every input line exactly once.

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test capture_translation`

Expected: FAIL because grouping and translation functions do not exist.

- [ ] **Step 2: Implement deterministic grouping**

Group lines only when their angles differ by at most 3°, vertical gap is at most 0.8 line heights and horizontal overlap exceeds 25%. Detect columns by x-gap clustering before line grouping. Keep confidence as the minimum of member lines.

- [ ] **Step 3: Implement JSON-structured block translation**

Send blocks as an array of `{id,text}` inside the untrusted source delimiter. Require the response to be an array with the exact same IDs and one `translatedText` per ID. Reject missing, duplicate or unknown IDs. Restore protected-term tokens after validation.

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test capture_translation`

Expected: PASS for two columns, rotated text, URL protection, invalid model output and tool request rejection.

- [ ] **Step 4: Commit structured image translation**

```bash
git add src-tauri/src/capture/layout.rs src-tauri/src/capture/translate.rs src-tauri/tests/capture_translation.rs PROJECT_LOG.txt
git commit -m "feat: translate OCR blocks with stable layout IDs"
```

### Task 6: Background restoration and translated text rendering

**Files:**
- Create: `src-tauri/src/capture/background.rs`
- Create: `src-tauri/src/capture/render.rs`
- Create: `src-tauri/tests/image_render.rs`
- Create: `tests/fixtures/fonts/NotoSans-Regular.ttf`
- Create: `tests/fixtures/fonts/LICENSE.txt`
- Modify: `src-tauri/Cargo.toml`
- Modify: `PROJECT_LOG.txt`

**Interfaces:**
- Consumes: `DecodedImage`, `TranslatedBlock`
- Produces: `RenderEngine::render -> RenderedImage`, `RenderWarning`

- [ ] **Step 1: Write failing image-difference tests**

Use fixed fixtures to assert that pixels outside expanded OCR masks are byte-identical, translated glyphs stay inside bounds, font size never drops below 8 px, alpha is preserved, and complex backgrounds emit `BackgroundApproximation`.

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test image_render`

Expected: FAIL because `RenderEngine` does not exist.

- [ ] **Step 2: Implement conservative background restoration**

Expand each text mask by 2 px, sample a 3 px exterior ring, compute color variance, fill with median color when variance is below 18, otherwise use edge-aware interpolation and emit a warning. Never alter pixels outside the expanded mask.

- [ ] **Step 3: Implement text fitting with cosmic-text**

Use binary search between source line height × 1.05 and 8 px, wrap at word boundaries, honor right-to-left direction, align to the source block, and select a fallback from bundled Noto fonts. If no size fits, render at 8 px, clip to the block and emit `TextOverflow`.

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test image_render`

Expected: PASS and generated fixture images match approved perceptual-hash thresholds.

- [ ] **Step 4: Record font license and commit**

```bash
git add src-tauri/src/capture/background.rs src-tauri/src/capture/render.rs src-tauri/tests/image_render.rs tests/fixtures/fonts src-tauri/Cargo.toml PROJECT_LOG.txt
git commit -m "feat: render translations into source image layout"
```

### Task 7: Capture result UI and safe export

**Files:**
- Create: `src/features/capture/CaptureResult.tsx`
- Create: `src/features/capture/CaptureResult.test.tsx`
- Create: `src/features/capture/captureApi.ts`
- Create: `src-tauri/src/capture/export.rs`
- Modify: `src-tauri/src/commands/capture.rs`
- Modify: `src/app/App.tsx`
- Test: `tests/e2e/capture-translation.spec.ts`
- Modify: `PROJECT_LOG.txt`
- Modify: `CHANGELOG.md`

**Interfaces:**
- Consumes: `CaptureJobResult`
- Produces: Tauri `translate_image`, `export_translated_image`, `<CaptureResult />`

- [ ] **Step 1: Write failing result-screen tests**

Test translated image preview, extracted translation, original/translated toggle, copy source, copy translation, save image, warning list, low-confidence highlighting and keyboard focus order.

Run: `pnpm test -- CaptureResult.test.tsx`

Expected: FAIL because the result component does not exist.

- [ ] **Step 2: Implement bounded export**

The save command accepts a job ID and user-chosen destination, not arbitrary bytes from the webview. It writes to a temporary sibling file, flushes, verifies PNG/JPEG decoding, then atomically renames. Existing destinations receive `_2`, `_3` naming unless the user explicitly confirms replacement.

- [ ] **Step 3: Implement the approved capture result layout**

Show the translated image on the left and extracted translation on the right, with save image, copy translation and copy source actions. Warnings link to the affected block and focus/highlight it. Provide an editable correction field that re-renders one block without re-running OCR.

- [ ] **Step 4: Run capture acceptance tests and commit**

Run:

```bash
pnpm test
pnpm exec playwright test tests/e2e/capture-translation.spec.ts
cargo test --manifest-path src-tauri/Cargo.toml capture
```

Expected: image import, screen capture mock, OCR, translation, render, copy and export all PASS; source hashes remain unchanged.

```bash
git add src/features/capture src-tauri/src/capture src-tauri/src/commands/capture.rs src/app/App.tsx tests/e2e/capture-translation.spec.ts PROJECT_LOG.txt CHANGELOG.md
git commit -m "feat: deliver image and screen translation"
```
