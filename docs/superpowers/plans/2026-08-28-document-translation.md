# Format-Preserving Document Translation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** DOCX, PPTX, XLSX와 PDF를 원본 구조와 배치를 최대한 유지하면서 번역하고 원본을 건드리지 않은 새 파일과 상세 검증 보고서를 만든다.

**Architecture:** 모든 문서 형식은 `검사 → 추출 → 구조화 번역 → 재조립 → 재열기 검증 → 원자적 저장` 파이프라인을 공유한다. OOXML 형식은 ZIP 항목과 XML 관계를 보존한 채 텍스트 노드만 수정하고 PDF는 텍스트/스캔 페이지별로 좌표 기반 오버레이를 적용한다.

**Tech Stack:** Rust, Tokio, zip, quick-xml, sha2, lopdf, pdfium-render, image pipeline from capture plan, Codex TranslationBackend, Cargo test

**Spec:** `docs/superpowers/specs/2026-08-28-smartcat-translate-design.md`

## Global Constraints

- DOCX, PPTX, XLSX, PDF를 모두 지원한다.
- 원본을 읽기 전용으로 열고 절대 덮어쓰지 않는다.
- 결과 이름은 `원본명_번역_대상언어.ext`이며 중복 시 번호를 붙인다.
- 저장 후 결과 문서를 다시 열어 구조와 요소 개수를 검사한다.
- 수식, URL, 파일 경로, 코드, 변수명과 보호 용어를 번역하지 않는다.
- 실패 위치와 서식 변화는 페이지, 슬라이드, 시트, 셀 또는 문서 경로로 보고한다.
- 손상 가능성이 있는 결과는 최종 경로로 이동하지 않는다.

---

### Task 1: Common document job and immutable output policy

**Files:**
- Create: `src-tauri/src/documents/mod.rs`
- Create: `src-tauri/src/documents/types.rs`
- Create: `src-tauri/src/documents/pipeline.rs`
- Create: `src-tauri/src/documents/output.rs`
- Create: `src/features/documents/types.ts`
- Create: `src-tauri/tests/document_output.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/Cargo.toml`
- Modify: `PROJECT_LOG.txt`

**Interfaces:**
- Produces: `DocumentFormat`, `DocumentOptions`, `Segment`, `DocumentWarning`, `DocumentReport`, `DocumentAdapter`, `OutputPolicy`

- [ ] **Step 1: Write failing output-safety tests**

```rust
#[test]
fn creates_language_suffix_and_never_reuses_existing_path() {
    let policy = OutputPolicy::new(fake_fs_with(&[
        "/docs/report.docx",
        "/docs/report_번역_한국어.docx",
    ]));
    let output = policy.next_path("/docs/report.docx", "한국어").unwrap();
    assert_eq!(output, path("/docs/report_번역_한국어_2.docx"));
}

#[tokio::test]
async fn failed_validation_does_not_publish_temp_output() {
    let result = pipeline(failing_validator()).run(job()).await;
    assert!(matches!(result, Err(DocumentError::ValidationFailed(_))));
    assert!(!fake_fs().exists("/docs/report_번역_한국어.docx"));
}
```

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test document_output`

Expected: FAIL because document contracts do not exist.

- [ ] **Step 2: Define the common adapter and report contracts**

```rust
#[async_trait::async_trait]
pub trait DocumentAdapter: Send + Sync {
    fn format(&self) -> DocumentFormat;
    async fn inspect(&self, source: &Path) -> Result<DocumentManifest, DocumentError>;
    async fn extract(&self, source: &Path, options: &DocumentOptions) -> Result<Vec<Segment>, DocumentError>;
    async fn rebuild(&self, source: &Path, segments: &[TranslatedSegment], temp: &Path)
        -> Result<Vec<DocumentWarning>, DocumentError>;
    async fn validate(&self, source: &Path, candidate: &Path, before: &DocumentManifest)
        -> Result<DocumentReport, DocumentError>;
}
```

`Segment` includes stable ID, logical location, text, style spans, protected ranges and translation context. `DocumentReport` includes source/output hashes, counts before/after, warnings, failures and `publishable`.

- [ ] **Step 3: Implement atomic output policy and staged pipeline**

Open source read-only, hash it before and after, write to a sibling `.smartcat-partial-<uuid>` file, flush and close, run validation, then rename to the reserved output path. A cancellation deletes the partial file. A source-hash change aborts with `SourceChanged`.

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test document_output`

Expected: PASS for duplicate names, cancellation, disk-full simulation, invalid candidate and source hash change.

- [ ] **Step 4: Commit the shared pipeline**

```bash
git add src-tauri/src/documents src-tauri/tests/document_output.rs src/features/documents/types.ts src-tauri/src/lib.rs src-tauri/Cargo.toml PROJECT_LOG.txt
git commit -m "feat: add immutable document translation pipeline"
```

### Task 2: Safe OOXML package reader and writer

**Files:**
- Create: `src-tauri/src/documents/ooxml/mod.rs`
- Create: `src-tauri/src/documents/ooxml/package.rs`
- Create: `src-tauri/src/documents/ooxml/xml.rs`
- Create: `src-tauri/tests/ooxml_package.rs`
- Create: `tests/fixtures/ooxml/minimal.docx`
- Modify: `src-tauri/Cargo.toml`
- Modify: `PROJECT_LOG.txt`

**Interfaces:**
- Produces: `OoxmlPackage::open`, `read_xml`, `replace_xml`, `write_preserving_entries`, `XmlTextNode`

- [ ] **Step 1: Write failing package preservation tests**

Create a minimal DOCX fixture containing `[Content_Types].xml`, relationships, document XML, styles and one binary image. Assert opening lists all entries, replacing only `word/document.xml` leaves every other entry byte-identical, preserves compression method and rejects `../` entry paths.

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test ooxml_package`

Expected: FAIL because `OoxmlPackage` does not exist.

- [ ] **Step 2: Implement bounded ZIP reading**

Reject encrypted entries, absolute paths, parent traversal, more than 20,000 entries, any expanded entry above 256 MB and total expansion above 1 GB. Preserve entry name, compression, Unix mode, comment and original bytes. Parse XML with DTD and external entity expansion disabled.

- [ ] **Step 3: Implement namespace-preserving XML text edits**

Use `quick_xml::Reader` and `Writer`; copy every event unchanged except target text events selected by exact part path and stable node ordinal. Preserve whitespace flags such as `xml:space="preserve"`. Escaping is delegated to the XML writer.

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test ooxml_package`

Expected: PASS for binary preservation, namespaces, whitespace, malformed XML, traversal and ZIP expansion limits.

- [ ] **Step 4: Commit the OOXML foundation**

```bash
git add src-tauri/src/documents/ooxml src-tauri/tests/ooxml_package.rs tests/fixtures/ooxml/minimal.docx src-tauri/Cargo.toml PROJECT_LOG.txt
git commit -m "feat: preserve OOXML package structure"
```

### Task 3: Structured segment translation and protected content

**Files:**
- Create: `src-tauri/src/documents/segments.rs`
- Create: `src-tauri/src/documents/translate.rs`
- Create: `src-tauri/tests/document_segments.rs`
- Modify: `src-tauri/src/documents/pipeline.rs`
- Modify: `PROJECT_LOG.txt`

**Interfaces:**
- Consumes: `TranslationBackend`, `Segment`, `TranslationProfile`
- Produces: `ProtectionMap`, `translate_segment_batch`, `TranslatedSegment`

- [ ] **Step 1: Write failing protection and response tests**

Test URL, email, Windows/macOS paths, inline code, Excel formulas, placeholders like `{name}`, user protected terms, exact segment IDs, retry on malformed JSON and cancellation between batches.

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test document_segments`

Expected: FAIL because segment translation does not exist.

- [ ] **Step 2: Implement reversible protection tokens**

Tokenize protected ranges as Unicode private-use strings containing a job-scoped random prefix and ordinal. Reject a response if a token is missing, duplicated, reordered where ordering is fixed, or newly invented. Restore exact original bytes after validation.

- [ ] **Step 3: Implement bounded structured batches**

Batch at paragraph/text-box boundaries with maximum 12,000 Unicode scalar values and 80 segments. Send `{documentContext, terminology, segments:[{id,text}]}` and require `{segments:[{id,translatedText}]}`. Preserve a rolling terminology table and a 1,000-character prior-context summary without logging either.

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test document_segments`

Expected: PASS for protection, duplicate IDs, partial response, retry limit, cancellation and terminology continuity.

- [ ] **Step 4: Commit segment translation**

```bash
git add src-tauri/src/documents/segments.rs src-tauri/src/documents/translate.rs src-tauri/src/documents/pipeline.rs src-tauri/tests/document_segments.rs PROJECT_LOG.txt
git commit -m "feat: translate structured document segments safely"
```

### Task 4: DOCX adapter

**Files:**
- Create: `src-tauri/src/documents/docx.rs`
- Create: `src-tauri/tests/docx_translation.rs`
- Create: `tests/fixtures/documents/docx-layout.docx`
- Modify: `src-tauri/src/documents/mod.rs`
- Modify: `PROJECT_LOG.txt`

**Interfaces:**
- Consumes: `OoxmlPackage`, `DocumentAdapter`, `TranslatedSegment`
- Produces: `DocxAdapter`

- [ ] **Step 1: Write failing DOCX fixture tests**

The fixture must contain styled split runs, table, numbered list, header, footer, footnote, endnote, hyperlink, field code, image and text box. Assert extraction combines sentence text while retaining run spans, includes all approved parts, excludes field code and hyperlink target, and maps every segment to exact text nodes.

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test docx_translation`

Expected: FAIL because `DocxAdapter` does not exist.

- [ ] **Step 2: Implement part discovery and extraction**

Discover main document, headers, footers, footnotes, endnotes and comments through relationship types. Walk paragraphs, tables and DrawingML text boxes. Join consecutive `w:t` nodes inside one logical paragraph while recording each run's character range and style ID. Exclude `w:instrText`, deleted text and relationship targets.

- [ ] **Step 3: Reinsert translated runs predictably**

Assign translated text to existing runs proportionally by source grapheme span, preferring sentence and whitespace boundaries. Keep all run property XML. If translation needs more spans, put remainder in the final run with `xml:space="preserve"`; if it needs fewer, set unused text nodes empty rather than removing runs.

- [ ] **Step 4: Validate and report layout risks**

Compare paragraph, table, image, relationship, header/footer, footnote/endnote counts. Detect a translated paragraph exceeding 1.8× source grapheme count and emit `PossibleReflow`. Reopen the ZIP and parse every modified XML part.

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test docx_translation`

Expected: PASS, all non-text entries byte-identical, source hash unchanged and translated output reopens.

- [ ] **Step 5: Commit DOCX support**

```bash
git add src-tauri/src/documents/docx.rs src-tauri/src/documents/mod.rs src-tauri/tests/docx_translation.rs tests/fixtures/documents/docx-layout.docx PROJECT_LOG.txt
git commit -m "feat: preserve DOCX layout during translation"
```

### Task 5: PPTX adapter

**Files:**
- Create: `src-tauri/src/documents/pptx.rs`
- Create: `src-tauri/tests/pptx_translation.rs`
- Create: `tests/fixtures/documents/pptx-layout.pptx`
- Modify: `src-tauri/src/documents/mod.rs`
- Modify: `PROJECT_LOG.txt`

**Interfaces:**
- Produces: `PptxAdapter`, `TextFitWarning`

- [ ] **Step 1: Write failing PPTX fixture tests**

The fixture contains title/body placeholders, free text box, grouped shape, table, chart labels, SmartArt fallback text, notes, image, animation relationship and an already auto-fit box. Assert approved visible text and notes are extracted, relationship/animation XML remains byte-identical and hidden slide inclusion follows options.

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test pptx_translation`

Expected: FAIL because `PptxAdapter` does not exist.

- [ ] **Step 2: Implement slide-order extraction**

Resolve presentation slide IDs to slide parts, then walk shapes in XML order. Extract `a:t` across paragraphs, tables, chart caches, diagram data and notes parts. Location is `slide:<n>/shape:<id>/paragraph:<n>` or `slide:<n>/notes/paragraph:<n>`.

- [ ] **Step 3: Preserve DrawingML and apply fit policy**

Replace only `a:t` values. Preserve `a:rPr`, `a:pPr`, shape transforms and relationships. Estimate overflow from text-box EMU dimensions, font size and translated grapheme width. Apply existing auto-fit first, then insert `a:normAutofit` with `fontScale` no lower than 70%; otherwise retain layout and emit `TextOverflow`.

- [ ] **Step 4: Validate and commit**

Compare slide, shape, image, animation, relationship and notes counts; parse every changed XML part.

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test pptx_translation`

Expected: PASS with exact non-text entry hashes and warnings only for the intentionally overflowing fixture shape.

```bash
git add src-tauri/src/documents/pptx.rs src-tauri/src/documents/mod.rs src-tauri/tests/pptx_translation.rs tests/fixtures/documents/pptx-layout.pptx PROJECT_LOG.txt
git commit -m "feat: preserve PPTX slides and notes during translation"
```

### Task 6: XLSX adapter

**Files:**
- Create: `src-tauri/src/documents/xlsx.rs`
- Create: `src-tauri/tests/xlsx_translation.rs`
- Create: `tests/fixtures/documents/xlsx-layout.xlsx`
- Modify: `src-tauri/src/documents/mod.rs`
- Modify: `PROJECT_LOG.txt`

**Interfaces:**
- Produces: `XlsxAdapter`, `CellLocation`

- [ ] **Step 1: Write failing XLSX fixture tests**

The fixture contains shared strings, inline strings, rich text, formulas, dates, URLs, merged cells, comments, hidden sheet, chart, conditional formatting and named ranges. Assert string cells extract in workbook/sheet/row/column order, formulas and URLs are protected, hidden/comment/sheet-name options work, and all numeric/date cells remain unchanged.

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test xlsx_translation`

Expected: FAIL because `XlsxAdapter` does not exist.

- [ ] **Step 2: Implement workbook relationships and string extraction**

Resolve sheet names and paths from workbook relationships. Extract shared-string `<si>` rich text and worksheet inline `<is>` text with use-site locations. When one shared string is used by cells requiring different context, clone the translated `<si>` and update only those cell indices.

- [ ] **Step 3: Preserve formulas and apply wrapping option**

Never modify `<f>`, numeric `<v>`, relationship hyperlinks or external links. For translated string cells, preserve style ID. If `wrap_text=true`, clone the cell style only when its alignment lacks wrapping, add wrapText, append the style and update the cell style index; never mutate a shared style used by unselected cells.

- [ ] **Step 4: Validate and commit**

Compare workbook sheet order, formula text, merged ranges, charts, drawings, named ranges and conditional formatting. Reopen and parse changed XML parts.

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test xlsx_translation`

Expected: PASS with formula hashes identical and source hash unchanged.

```bash
git add src-tauri/src/documents/xlsx.rs src-tauri/src/documents/mod.rs src-tauri/tests/xlsx_translation.rs tests/fixtures/documents/xlsx-layout.xlsx PROJECT_LOG.txt
git commit -m "feat: preserve XLSX formulas and formatting during translation"
```

### Task 7: PDF classification, OCR and translated overlay

**Files:**
- Create: `src-tauri/src/documents/pdf/mod.rs`
- Create: `src-tauri/src/documents/pdf/classify.rs`
- Create: `src-tauri/src/documents/pdf/extract.rs`
- Create: `src-tauri/src/documents/pdf/rebuild.rs`
- Create: `src-tauri/tests/pdf_translation.rs`
- Create: `tests/fixtures/documents/text-layout.pdf`
- Create: `tests/fixtures/documents/scanned-layout.pdf`
- Modify: `src-tauri/src/documents/mod.rs`
- Modify: `src-tauri/Cargo.toml`
- Modify: `PROJECT_LOG.txt`
- Modify: `DECISIONS.txt`

**Interfaces:**
- Consumes: `OcrEngine`, capture `RenderEngine`, `DocumentAdapter`
- Produces: `PdfAdapter`, `PdfPageKind`, `PdfBlock`

- [ ] **Step 1: Write failing classification and geometry tests**

Assert the text fixture is `Text`, scanned fixture is `Scanned`, mixed documents classify per page, extracted coordinates convert PDF bottom-left points to normalized top-left coordinates, page rotation is honored and encrypted PDFs return `PasswordRequired` without logging the path.

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test pdf_translation`

Expected: FAIL because PDF modules do not exist.

- [ ] **Step 2: Implement deterministic page classification**

Render each page at 144 DPI and extract Unicode text with bounds. Classify as text when at least 20 non-whitespace characters exist and text bounds cover at least 0.5% of page area; otherwise classify as scanned. A user override may force OCR per page.

- [ ] **Step 3: Implement extraction and translation blocks**

Text pages group extracted characters into lines and blocks using the same capture layout rules. Scanned pages pass rendered pixels to `OcrEngine`. Preserve page number, media/crop box, rotation, source font hints and normalized bounds.

- [ ] **Step 4: Implement non-destructive PDF rebuild**

Copy every original page and resource. Add one overlay content stream that masks source text bounds using the capture background estimator and draws translated text with embedded Noto fonts. Do not remove original content streams. For complex backgrounds, rasterize only the affected page at 300 DPI, render translations into pixels and replace that page while emitting `RasterizedPage`.

- [ ] **Step 5: Validate PDF output**

Reopen the output, compare page count, boxes, rotation, annotations and attachments, render every page, verify dimensions, and ensure source hash is unchanged. Report substituted fonts, rasterized pages, low-confidence OCR and overflow.

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test pdf_translation`

Expected: PASS for text, scanned, mixed, rotated, encrypted and malformed fixtures.

- [ ] **Step 6: Record PDF runtime licensing and commit**

Record the exact PDFium distribution source, version, platform binaries, SHA-256 values and license in `DECISIONS.txt`; ensure CI verifies them before packaging.

```bash
git add src-tauri/src/documents/pdf src-tauri/src/documents/mod.rs src-tauri/tests/pdf_translation.rs tests/fixtures/documents/*.pdf src-tauri/Cargo.toml PROJECT_LOG.txt DECISIONS.txt
git commit -m "feat: translate text and scanned PDF pages"
```

### Task 8: Document options, progress, report and export UI

**Files:**
- Create: `src/features/documents/documentApi.ts`
- Create: `src/features/documents/DocumentWorkspace.tsx`
- Create: `src/features/documents/DocumentOptions.tsx`
- Create: `src/features/documents/DocumentReport.tsx`
- Create: `src/features/documents/DocumentWorkspace.test.tsx`
- Create: `src-tauri/src/commands/documents.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src/app/App.tsx`
- Test: `tests/e2e/document-translation.spec.ts`
- Modify: `PROJECT_LOG.txt`
- Modify: `CHANGELOG.md`

**Interfaces:**
- Produces: Tauri `inspect_document`, `start_document_translation`, `cancel_document_translation`, `open_document_result`

- [ ] **Step 1: Write failing UI tests**

Test file inspection, format badge, default translation scope, optional comments/hidden content/sheet names, destination preview, staged progress, cancel, retry, completed path, warning filters and opening the result. Verify no full local path appears in accessibility text until the user expands file details.

Run: `pnpm test -- DocumentWorkspace.test.tsx`

Expected: FAIL because document components do not exist.

- [ ] **Step 2: Implement job events without document content**

```rust
#[derive(Clone, serde::Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum DocumentJobEvent {
    Progress { job_id: Uuid, stage: DocumentStage, completed: u32, total: u32 },
    Warning { job_id: Uuid, warning: DocumentWarning },
    Completed { job_id: Uuid, report: DocumentReport },
    Failed { job_id: Uuid, code: String, location: Option<String> },
}
```

Never include segment text in events or logs. Display only filename, format, item counts and logical locations.

- [ ] **Step 3: Implement the document workspace**

Accept drag/drop and file dialog. Show source/target language, quality, default and optional scopes, output directory, progress stages and report. Report rows link to page/slide/sheet/cell locations and explain reflow, overflow, fallback font, OCR confidence and rasterized page warnings.

- [ ] **Step 4: Run format acceptance tests**

The E2E harness translates fixed phrases in all four fixtures through a fake backend. Assert new files exist, originals retain SHA-256, outputs reopen, expected element counts match and warnings are visible.

Run:

```bash
pnpm test
pnpm exec playwright test tests/e2e/document-translation.spec.ts
cargo test --manifest-path src-tauri/Cargo.toml documents
```

Expected: all PASS for DOCX, PPTX, XLSX and both PDF classes.

- [ ] **Step 5: Record and commit the document milestone**

```bash
git add src/features/documents src-tauri/src/commands/documents.rs src-tauri/src/commands/mod.rs src-tauri/src/lib.rs src/app/App.tsx tests/e2e/document-translation.spec.ts PROJECT_LOG.txt CHANGELOG.md
git commit -m "feat: deliver format-preserving document translation"
```
