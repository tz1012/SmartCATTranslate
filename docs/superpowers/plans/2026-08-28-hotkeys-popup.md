# Global Hotkeys and Quick Translation Popup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 다른 프로그램에서 구성 가능한 전역 단축키와 연속 입력으로 선택 텍스트를 번역하고 작은 팝업에 결과를 표시한다.

**Architecture:** 일반 조합은 Tauri 전역 단축키 플러그인으로 등록하고 연속 입력은 플랫폼별 키 이벤트 소스를 공통 상태기계에 공급한다. 충돌 분석은 실제 등록 시험, 예약 조합, 실행 중 앱과 알려진 단축키 카탈로그를 합성하며 선택 텍스트 획득은 클립보드를 스냅샷 후 복원한다.

**Tech Stack:** Tauri 2 global-shortcut/clipboard/window-state plugins, React, TypeScript, Rust, Windows API, macOS CoreGraphics, Vitest, Cargo test

**Spec:** `docs/superpowers/specs/2026-08-28-smartcat-translate-design.md`

## Global Constraints

- 단일 조합, 동일 조합 반복과 Ctrl+C 다음 C 같은 연속 입력을 지원한다.
- 충돌 시 등록 차단, 가능한 원인 표시와 대체 단축키 추천이 기본이다.
- 사용자가 명시적으로 선택한 경우에만 경고 후 강제 등록한다.
- 앱별 차단 목록을 적용한다.
- 선택 텍스트를 얻은 뒤 기존 클립보드를 복원한다.
- 팝업은 키보드만으로 닫기, 고정, 복사와 전체 창 열기가 가능해야 한다.

---

### Task 1: Hotkey domain model and parser

**Files:**
- Create: `src-tauri/src/hotkeys/mod.rs`
- Create: `src-tauri/src/hotkeys/types.rs`
- Create: `src-tauri/src/hotkeys/parser.rs`
- Create: `src/features/hotkeys/types.ts`
- Test: `src-tauri/src/hotkeys/parser.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `PROJECT_LOG.txt`

**Interfaces:**
- Produces: `KeyCode`, `Modifiers`, `Chord`, `HotkeyBinding`, `Trigger`, `parse_trigger(&str) -> Result<Trigger, HotkeyError>`

- [ ] **Step 1: Write failing parser tests**

```rust
#[test]
fn parses_single_and_sequence_triggers() {
    assert_eq!(parse_trigger("Ctrl+Shift+C").unwrap(), trigger("Ctrl+Shift+C"));
    assert_eq!(parse_trigger("Ctrl+C, C").unwrap(), sequence("Ctrl+C", "C", 650));
}

#[test]
fn rejects_modifier_only_and_more_than_four_steps() {
    assert!(parse_trigger("Ctrl").is_err());
    assert!(parse_trigger("A, B, C, D, E").is_err());
}
```

Run: `cargo test --manifest-path src-tauri/Cargo.toml hotkeys::parser`

Expected: FAIL because the parser does not exist.

- [ ] **Step 2: Define normalized trigger types**

```rust
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Deserialize, serde::Serialize)]
pub struct Chord { pub modifiers: Modifiers, pub key: KeyCode }

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Trigger {
    Chord { chord: Chord },
    Sequence { steps: Vec<Chord>, timeout_ms: u64 },
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyBinding {
    pub id: uuid::Uuid,
    pub trigger: Trigger,
    pub action: HotkeyAction,
    pub profile_id: uuid::Uuid,
    pub force: bool,
}
```

Normalize `Control` to `Ctrl`, `CommandOrControl` to the current platform modifier, sort modifiers as Ctrl/Alt/Shift/Meta, trim whitespace, set sequence timeout to 650 ms, and cap sequences at four chords.

- [ ] **Step 3: Implement and verify the parser**

Run: `cargo test --manifest-path src-tauri/Cargo.toml hotkeys::parser`

Expected: PASS for valid Windows/macOS aliases, duplicate modifiers, invalid keys, whitespace and maximum steps.

- [ ] **Step 4: Commit the hotkey contract**

```bash
git add src-tauri/src/hotkeys src-tauri/src/lib.rs src/features/hotkeys/types.ts PROJECT_LOG.txt
git commit -m "feat: define configurable hotkey triggers"
```

### Task 2: Deterministic sequence engine

**Files:**
- Create: `src-tauri/src/hotkeys/sequence.rs`
- Test: `src-tauri/src/hotkeys/sequence.rs`
- Modify: `PROJECT_LOG.txt`

**Interfaces:**
- Consumes: `Chord`, `Trigger`
- Produces: `SequenceEngine::new`, `SequenceEngine::on_key(chord, at) -> Vec<Uuid>`

- [ ] **Step 1: Write failing state-machine tests with controlled time**

```rust
#[test]
fn fires_ctrl_c_then_c_inside_timeout() {
    let id = Uuid::new_v4();
    let mut engine = SequenceEngine::new(vec![binding(id, "Ctrl+C, C", 650)]);
    assert!(engine.on_key(chord("Ctrl+C"), ms(0)).is_empty());
    assert_eq!(engine.on_key(chord("C"), ms(400)), vec![id]);
}

#[test]
fn resets_after_timeout_or_wrong_key() {
    let id = Uuid::new_v4();
    let mut engine = SequenceEngine::new(vec![binding(id, "Ctrl+C, C", 650)]);
    engine.on_key(chord("Ctrl+C"), ms(0));
    assert!(engine.on_key(chord("C"), ms(651)).is_empty());
    engine.on_key(chord("Ctrl+C"), ms(800));
    assert!(engine.on_key(chord("V"), ms(900)).is_empty());
}
```

Run: `cargo test --manifest-path src-tauri/Cargo.toml hotkeys::sequence`

Expected: FAIL because `SequenceEngine` does not exist.

- [ ] **Step 2: Implement prefix matching without swallowing keystrokes**

Store one cursor per binding: next step index and last timestamp. Every incoming key updates matching cursors, resets expired or mismatched cursors, and emits binding IDs whose final step matched. The engine observes events only; it never cancels or modifies the host application's key event.

```rust
pub fn on_key(&mut self, chord: Chord, at: Duration) -> Vec<Uuid> {
    self.bindings.iter().filter_map(|binding| {
        let cursor = self.cursors.entry(binding.id).or_default();
        cursor.advance(binding, &chord, at)
    }).collect()
}
```

Run: `cargo test --manifest-path src-tauri/Cargo.toml hotkeys::sequence`

Expected: PASS for overlapping prefixes, repeated chords, timeout boundaries and independent bindings.

- [ ] **Step 3: Commit the sequence engine**

```bash
git add src-tauri/src/hotkeys/sequence.rs PROJECT_LOG.txt
git commit -m "feat: recognize non-blocking hotkey sequences"
```

### Task 3: Conflict analyzer and alternative recommender

**Files:**
- Create: `src-tauri/resources/shortcut-catalog.json`
- Create: `src-tauri/src/hotkeys/conflicts.rs`
- Create: `src-tauri/tests/hotkey_conflicts.rs`
- Modify: `DECISIONS.txt`
- Modify: `PROJECT_LOG.txt`

**Interfaces:**
- Produces: `RegistrationProbe`, `AppInspector`, `ConflictAnalyzer::analyze`, `ConflictReport`, `suggest_alternatives`

- [ ] **Step 1: Write failing conflict classification tests**

```rust
#[test]
fn distinguishes_confirmed_and_possible_conflicts() {
    let analyzer = analyzer(probe(false), running_apps(&["chrome.exe"]));
    let report = analyzer.analyze(&trigger("Ctrl+L"));
    assert_eq!(report.level, ConflictLevel::Confirmed);
    assert!(report.causes.iter().any(|cause| cause.application == Some("Google Chrome".into())));
    assert!(report.alternatives.len() >= 3);
}

#[test]
fn never_claims_an_unknown_owner() {
    let report = analyzer(probe(false), running_apps(&[])).analyze(&trigger("Ctrl+Alt+9"));
    assert_eq!(report.causes[0].description, "다른 프로그램이 사용 중일 수 있습니다.");
    assert_eq!(report.causes[0].application, None);
}
```

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test hotkey_conflicts`

Expected: FAIL because analyzer types do not exist.

- [ ] **Step 2: Add a source-attributed shortcut catalog**

Each catalog row contains `platform`, `application`, `processNames`, `trigger`, `feature`, `sourceUrl`, and `verifiedAt`. Seed Windows/macOS reserved combinations and Chrome, Edge, Word, PowerPoint and Excel entries used by tests. The catalog build test rejects missing URLs, dates older than 18 months and duplicate application/trigger rows.

- [ ] **Step 3: Implement layered conflict analysis**

```rust
pub enum ConflictLevel { None, Possible, Confirmed }

pub struct ConflictReport {
    pub level: ConflictLevel,
    pub causes: Vec<ConflictCause>,
    pub alternatives: Vec<Trigger>,
    pub can_force: bool,
}
```

Confirmed means OS registration probe failed or an OS-reserved trigger matched. Possible means a running app matched a catalog entry. Rank alternatives by same key with additional modifiers, then neighboring function keys, and reject every candidate that fails the same analyzer.

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test hotkey_conflicts`

Expected: PASS with at least three non-conflicting alternatives for every seeded collision.

- [ ] **Step 4: Record catalog sources and commit**

```bash
git add src-tauri/resources/shortcut-catalog.json src-tauri/src/hotkeys/conflicts.rs src-tauri/tests/hotkey_conflicts.rs DECISIONS.txt PROJECT_LOG.txt
git commit -m "feat: analyze and explain hotkey conflicts"
```

### Task 4: Platform key-event sources and foreground application inspection

**Files:**
- Create: `src-tauri/src/platform/mod.rs`
- Create: `src-tauri/src/platform/windows/mod.rs`
- Create: `src-tauri/src/platform/windows/keyboard.rs`
- Create: `src-tauri/src/platform/windows/foreground.rs`
- Create: `src-tauri/src/platform/macos/mod.rs`
- Create: `src-tauri/src/platform/macos/keyboard.rs`
- Create: `src-tauri/src/platform/macos/foreground.rs`
- Create: `src-tauri/src/hotkeys/native.rs`
- Modify: `src-tauri/Cargo.toml`
- Modify: `PROJECT_LOG.txt`

**Interfaces:**
- Produces: `KeyEventSource::start(Sender<KeyEvent>)`, `ForegroundAppProvider::current() -> AppIdentity`

- [ ] **Step 1: Write the platform-neutral contract test**

Create a fake `KeyEventSource` that sends Ctrl+C and C and assert the native controller emits one binding ID while `ForegroundAppProvider` returns the fake executable and bundle identifier.

Run: `cargo test --manifest-path src-tauri/Cargo.toml hotkeys::native`

Expected: FAIL because the traits do not exist.

- [ ] **Step 2: Implement Windows adapters**

Use `SetWindowsHookExW(WH_KEYBOARD_LL)` on a dedicated message-loop thread, translate key-down events to normalized `KeyEvent`, and always call `CallNextHookEx`. Use `GetForegroundWindow`, `GetWindowThreadProcessId`, and `QueryFullProcessImageNameW` for the foreground executable. Unhook on app exit.

- [ ] **Step 3: Implement macOS adapters**

Use a listen-only `CGEventTapCreate` for key-down events, never return a modified event, and surface `AccessibilityPermissionRequired` when the tap cannot be created. Resolve the frontmost app with `NSWorkspace.frontmostApplication`, returning localized name and bundle identifier.

- [ ] **Step 4: Verify cfg isolation and commit**

Run on Windows:

```bash
cargo check --manifest-path src-tauri/Cargo.toml --target x86_64-pc-windows-msvc
```

Run in GitHub Actions on macOS:

```bash
cargo check --manifest-path src-tauri/Cargo.toml --target aarch64-apple-darwin
cargo check --manifest-path src-tauri/Cargo.toml --target x86_64-apple-darwin
```

Expected: platform modules compile only for their targets and shared tests PASS.

```bash
git add src-tauri/src/platform src-tauri/src/hotkeys/native.rs src-tauri/Cargo.toml PROJECT_LOG.txt
git commit -m "feat: observe hotkeys on Windows and macOS"
```

### Task 5: Clipboard-safe selected-text acquisition and app blocklist

**Files:**
- Create: `src-tauri/src/hotkeys/clipboard.rs`
- Create: `src-tauri/src/hotkeys/blocklist.rs`
- Create: `src-tauri/tests/clipboard_guard.rs`
- Modify: `src-tauri/src/hotkeys/mod.rs`
- Modify: `PROJECT_LOG.txt`

**Interfaces:**
- Consumes: `ForegroundAppProvider`
- Produces: `ClipboardPort`, `CopySynthesizer`, `ClipboardGuard::capture_selected_text`, `Blocklist::allows`

- [ ] **Step 1: Write failing clipboard restoration tests**

```rust
#[tokio::test]
async fn restores_text_and_non_text_clipboard_formats_after_capture() {
    let clipboard = FakeClipboard::with_items(vec![html("<b>old</b>"), text("old")]);
    let copier = FakeCopySynthesizer::selecting("selected text");
    let captured = ClipboardGuard::new(&clipboard, &copier)
        .capture_selected_text(Duration::from_millis(500)).await.unwrap();
    assert_eq!(captured, "selected text");
    assert_eq!(clipboard.items(), vec![html("<b>old</b>"), text("old")]);
}
```

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test clipboard_guard`

Expected: FAIL because `ClipboardGuard` does not exist.

- [ ] **Step 2: Implement snapshot, generation wait and restoration**

Take a clipboard snapshot, invoke platform copy synthesis only when the trigger did not already include a copy step, wait for the clipboard generation counter to change up to 500 ms, read plain text, and restore every captured clipboard format in a `Drop` guard. Return `NoSelection` for blank or unchanged text.

- [ ] **Step 3: Apply app blocklist before clipboard access**

```rust
#[derive(Clone, serde::Deserialize, serde::Serialize)]
pub struct BlockedApp { pub platform: Platform, pub executable: Option<String>, pub bundle_id: Option<String> }

impl Blocklist {
    pub fn allows(&self, app: &AppIdentity) -> bool {
        !self.entries.iter().any(|entry| entry.matches(app))
    }
}
```

The hotkey controller checks `allows` before reading the clipboard or opening a window. Record a sanitized `blocked` audit event containing only the catalog application name.

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test clipboard_guard`

Expected: PASS for text, HTML+text, timeout, empty selection, copy failure and blocked app.

- [ ] **Step 4: Commit clipboard safety**

```bash
git add src-tauri/src/hotkeys src-tauri/tests/clipboard_guard.rs PROJECT_LOG.txt
git commit -m "feat: capture selections without changing clipboard"
```

### Task 6: Hotkey settings and conflict UI

**Files:**
- Create: `src/features/hotkeys/hotkeyApi.ts`
- Create: `src/features/hotkeys/HotkeyRecorder.tsx`
- Create: `src/features/hotkeys/HotkeySettings.tsx`
- Create: `src/features/hotkeys/HotkeySettings.test.tsx`
- Create: `src-tauri/src/commands/hotkeys.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `PROJECT_LOG.txt`

**Interfaces:**
- Consumes: `ConflictAnalyzer`, `HotkeyBinding`
- Produces: Tauri `analyze_hotkey`, `save_hotkey`, `list_hotkeys`, `list_blocked_apps`

- [ ] **Step 1: Write failing conflict-dialog tests**

Test that a confirmed conflict disables `저장`, shows application and feature, renders three alternative buttons, selecting an alternative re-runs analysis, and `경고 후 강제 등록` requires a second explicit confirmation.

Run: `pnpm test -- HotkeySettings.test.tsx`

Expected: FAIL because settings components do not exist.

- [ ] **Step 2: Implement keyboard recording without triggering active shortcuts**

While `HotkeyRecorder` is focused, call `suspend_hotkeys(true)`, collect up to four chord key-up events, render them as Korean labels, and call `suspend_hotkeys(false)` on accept, cancel and component unmount. Never intercept Tab or Escape; Escape cancels recording.

- [ ] **Step 3: Implement analysis, alternatives and blocklist editing**

Use `aria-describedby` to bind the error explanation to the recorder. Alternatives are real buttons labeled with normalized triggers. Blocklist entries show application name, executable/bundle ID and separate toggles for keyboard and floating popup behavior.

Run: `pnpm test -- HotkeySettings.test.tsx && cargo test --manifest-path src-tauri/Cargo.toml hotkeys`

Expected: PASS.

- [ ] **Step 4: Commit settings**

```bash
git add src/features/hotkeys src-tauri/src/commands src-tauri/src/lib.rs PROJECT_LOG.txt
git commit -m "feat: configure hotkeys and resolve conflicts"
```

### Task 7: Quick translation popup

**Files:**
- Create: `src/features/translation/QuickPopup.tsx`
- Create: `src/features/translation/QuickPopup.test.tsx`
- Create: `src-tauri/src/platform/speech.rs`
- Create: `src-tauri/src/platform/windows/speech.rs`
- Create: `src-tauri/src/platform/macos/speech.rs`
- Create: `src-tauri/src/commands/windows.rs`
- Modify: `src-tauri/tauri.conf.json`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src/styles.css`
- Test: `tests/e2e/quick-popup.spec.ts`
- Modify: `PROJECT_LOG.txt`
- Modify: `CHANGELOG.md`

**Interfaces:**
- Consumes: selected text, `translate_text`, translation profile ID
- Produces: `show_quick_popup`, `<QuickPopup />`

- [ ] **Step 1: Write failing popup behavior tests**

Test loading, source/result panes, streamed result, copy, listen, pin, Escape close, focus return, `전체 창에서 열기`, and an error state that does not expose source text.

Run: `pnpm test -- QuickPopup.test.tsx`

Expected: FAIL because `QuickPopup` does not exist.

- [ ] **Step 2: Configure a reusable popup window**

Create hidden label `quick-popup`, 560×360 logical pixels, always-on-top only while visible, no taskbar entry, and platform-native shadow. Position near the active monitor cursor with 16 px inset and clamp to work area. Reuse one window instead of creating a window per trigger.

- [ ] **Step 3: Connect hotkey activation to translation**

Flow: binding fires → foreground app passes blocklist → selected text acquired → popup receives source/profile → popup opens in loading state → translation streams → result actions enable. When unpinned, blur closes after 150 ms unless an action button owns focus.

Implement `SpeechPort::speak(text, language_tag)` and `stop()`. Windows uses `Windows.Media.SpeechSynthesis.SpeechSynthesizer`; macOS uses `AVSpeechSynthesizer`. The popup's `듣기` button is enabled only when the platform reports an installed matching voice, changes to `중지` while speaking, and never sends speech text to another network service.

- [ ] **Step 4: Run end-to-end acceptance tests**

The E2E harness supplies a fake foreground app, clipboard and Codex transport. Assert `Ctrl+C, C` opens one popup, the original multi-format clipboard is restored, a blocked app opens nothing, conflict force requires confirmation, and Escape closes.

Run:

```bash
pnpm test
pnpm exec playwright test tests/e2e/quick-popup.spec.ts
cargo test --manifest-path src-tauri/Cargo.toml hotkeys
```

Expected: all PASS.

- [ ] **Step 5: Record and commit the hotkey milestone**

Update `PROJECT_LOG.txt` with Windows test evidence and macOS CI compile evidence. Add quick popup, sequential hotkeys, conflict explanations and blocklist to `CHANGELOG.md`.

```bash
git add src src-tauri tests PROJECT_LOG.txt CHANGELOG.md
git commit -m "feat: deliver global quick translation popup"
```

### Task 8: Tray/menu-bar residency, launch at login and close behavior

**Files:**
- Create: `src-tauri/src/lifecycle.rs`
- Create: `src-tauri/src/commands/lifecycle.rs`
- Create: `src/features/settings/LifecycleSettings.test.tsx`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/tauri.conf.json`
- Modify: `src/features/settings/SettingsView.tsx`
- Modify: `PROJECT_LOG.txt`
- Modify: `CHANGELOG.md`

**Interfaces:**
- Consumes: `AppSettings.launch_at_login`, `CloseBehavior`, `QuickAccessPosition`
- Produces: tray/menu-bar commands, `set_launch_at_login`, `set_close_behavior`

- [ ] **Step 1: Write failing lifecycle tests**

Test `KeepInTray` hides the main window, `Quit` exits after unregistering hotkeys and stopping Codex, `AskEveryTime` displays three choices, tray/menu-bar `빠른 번역` opens the configured popup/main window, and launch-at-login toggle calls the autostart plugin exactly once.

Run: `pnpm test -- LifecycleSettings.test.tsx && cargo test --manifest-path src-tauri/Cargo.toml lifecycle`

Expected: FAIL because lifecycle commands do not exist.

- [ ] **Step 2: Implement one cross-platform residency controller**

Create a system tray on Windows and menu-bar item on macOS with `빠른 번역`, `전체 창 열기`, `단축키 일시 중지`, `설정`, and `종료`. `종료` unregisters every hotkey, cancels speech, checkpoints recoverable jobs, stops App Server and then exits. Window-close interception follows `CloseBehavior` and never runs during updater-requested restart.

- [ ] **Step 3: Implement launch-at-login and settings synchronization**

Use the Tauri autostart plugin with the installed app executable only; development builds display `설치된 앱에서만 사용할 수 있습니다`. Roll back the settings switch when the OS call fails. Propagate close behavior and quick-access position immediately without restart.

Run: `pnpm test -- LifecycleSettings.test.tsx && cargo test --manifest-path src-tauri/Cargo.toml lifecycle`

Expected: PASS.

- [ ] **Step 4: Record and commit lifecycle behavior**

```bash
git add src-tauri/src/lifecycle.rs src-tauri/src/commands/lifecycle.rs src-tauri/src/commands/mod.rs src-tauri/src/lib.rs src-tauri/tauri.conf.json src/features/settings PROJECT_LOG.txt CHANGELOG.md
git commit -m "feat: keep translation available from tray and menu bar"
```
