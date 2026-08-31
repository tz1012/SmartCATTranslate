# Compact Shell and Runtime Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver an installed Windows build with a compact DeepL-inspired shell, overlay menu, efficient settings, working ChatGPT browser login, reliable capture Enter/Escape, and no console window.

**Architecture:** Keep `App` as the view owner, extract the permanent top bar and transient menu overlay into focused React components, and simplify each workspace so navigation exists in one place. Fix runtime defects at their owning boundaries: Windows entry subsystem, Tauri opener handoff, and capture overlay focus/key routing.

**Tech Stack:** Tauri 2, React 19, TypeScript, Rust, Vitest/Testing Library, Windows MSI/NSIS packaging.

**Spec:** `docs/superpowers/specs/2026-08-31-compact-shell-runtime-fixes-design.md`

## Global Constraints

- Main window default size is 1000×700, minimum size is 760×520, and aspect ratio is unrestricted.
- Hamburger content is an anchored overlay; it never pushes or resizes the workspace and starts closed.
- Official ChatGPT URL validation, managed login cancellation, sanitized errors, clipboard preservation, blocked-app checks, private temporary storage, and secret-free diagnostics remain intact.
- Do not edit `sources/`, remove user data, remove existing tests, add failure injection, or run the long full regression suite.
- Keep source, toolchain, records, and installer outputs on drive D where possible.
- Record decisions, changes, focused checks, hashes, and remaining limitations in `PROJECT_LOG.txt`, `DECISIONS.txt`, and `CHANGELOG.md`.

---

### Task 1: Windows GUI entry and window size policy

**Files:**
- Modify: `src-tauri/src/main.rs`
- Modify: `src-tauri/tauri.conf.json`
- Modify: `src-tauri/Cargo.toml` only if a binary inspection helper is already available as a dev dependency; do not add a new production dependency.

**Interfaces:**
- Consumes: Tauri desktop `smartcat_translate::run()` entry point.
- Produces: release Windows GUI executable and a main window bounded by `minWidth: 760`, `minHeight: 520`.

- [ ] **Step 1: Add the non-debug Windows GUI subsystem declaration**

```rust
#![cfg_attr(all(not(debug_assertions), target_os = "windows"), windows_subsystem = "windows")]

fn main() {
    smartcat_translate::run();
}
```

- [ ] **Step 2: Add only the main-window minimum dimensions**

```json
{
  "label": "main",
  "title": "SmartCAT Translate",
  "width": 1000,
  "height": 700,
  "minWidth": 760,
  "minHeight": 520
}
```

- [ ] **Step 3: Run focused static checks**

Run: `pnpm exec tauri build --debug --no-bundle` only if no cargo/rustc/link process is running; otherwise defer this gate to Task 6.

Expected: the desktop target compiles and Tauri accepts the window schema.

- [ ] **Step 4: Commit**

```text
git add src-tauri/src/main.rs src-tauri/tauri.conf.json
git commit -m "fix: launch Windows build without a console"
```

### Task 2: ChatGPT browser handoff

**Files:**
- Modify: `src-tauri/src/commands/account.rs`
- Test: `src-tauri/tests/codex_auth.rs`

**Interfaces:**
- Consumes: `start_login_and_open(&AccountService, &dyn BrowserOpener)` and validated `url::Url`.
- Produces: `TauriBrowserOpener<R> { app: AppHandle<R> }` using `OpenerExt::open_url` through the plugin instance already initialized in `lib.rs`.

- [ ] **Step 1: Preserve the existing abstraction test for successful and failed opener handoff**

Confirm these focused behaviors remain represented in `src-tauri/tests/codex_auth.rs`:

```rust
let start = start_login_and_open(&service, &opener).await;
assert!(start.is_ok());
assert_eq!(
    opener.opened_urls(),
    vec!["https://auth.chatgpt.com/oauth/authorize?client_id=smartcat-test"]
);
```

Do not add another synthetic failure suite; the existing opener-failure and URL-policy tests cover cleanup and sanitization.

- [ ] **Step 2: Route the command through the initialized application plugin**

```rust
use tauri_plugin_opener::OpenerExt;

pub async fn start_chatgpt_login<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
) -> Result<StartLoginResult, String> {
    let service = state.account_service().await.ok_or(SERVICE_UNAVAILABLE)?;
    let opener = TauriBrowserOpener { app };
    start_login_and_open(service.as_ref(), &opener).await.map_err(|error| error_code(&error))?;
    Ok(StartLoginResult::BrowserOpened)
}

struct TauriBrowserOpener<R: Runtime> { app: AppHandle<R> }

impl<R: Runtime> BrowserOpener for TauriBrowserOpener<R> {
    fn open(&self, url: &url::Url) -> Result<(), BrowserOpenError> {
        self.app.opener().open_url(url.as_str(), None::<&str>).map_err(|_| BrowserOpenError)
    }
}
```

- [ ] **Step 3: Run only the focused authentication test target**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test codex_auth`

Expected: existing managed-login, URL-policy, and opener tests pass without printing auth URLs or login identifiers.

- [ ] **Step 4: Commit**

```text
git add src-tauri/src/commands/account.rs
git commit -m "fix: open ChatGPT login with the app opener"
```

### Task 3: Capture overlay keyboard reliability

**Files:**
- Modify: `src/features/capture/CaptureOverlay.tsx`
- Create: `src/features/capture/CaptureOverlay.test.tsx`
- Modify: `src-tauri/src/commands/capture.rs` only if runtime inspection proves `set_focus()` is called before a window becomes visible; keep backend changes limited to show/focus ordering.

**Interfaces:**
- Consumes: `completeScreenCapture(sessionId, selection)` and `cancelScreenCapture(sessionId)`.
- Produces: `isConfirmKey(event: KeyboardEvent): boolean`, `isCancelKey(event: KeyboardEvent): boolean`, and capture-phase key routing bound to the active overlay document.

- [ ] **Step 1: Add two happy-path component tests**

```tsx
it('confirms a valid selection with Enter', async () => {
  render(<CaptureOverlay />);
  await seedDescriptorAndSelection();
  fireEvent.keyDown(document, { key: 'Enter', code: 'Enter' });
  expect(completeScreenCapture).toHaveBeenCalledWith('session-1', validSelection);
});

it('cancels from Escape before a selection exists', async () => {
  render(<CaptureOverlay />);
  await seedDescriptor();
  fireEvent.keyDown(document, { key: 'Escape', code: 'Escape' });
  expect(cancelScreenCapture).toHaveBeenCalledWith('session-1');
});
```

- [ ] **Step 2: Verify the focused tests expose the current focus/key routing gap**

Run: `pnpm exec vitest run src/features/capture/CaptureOverlay.test.tsx`

Expected before the change: at least one new assertion fails because the handler is only attached to `window` bubble phase or because focus is not explicitly reclaimed.

- [ ] **Step 3: Implement capture-phase routing and explicit focus**

```tsx
const overlayRef = useRef<HTMLElement>(null);

useEffect(() => {
  const key = (event: KeyboardEvent) => {
    if (event.repeat) return;
    if (event.key === 'Escape' || event.key === 'Esc') { event.preventDefault(); cancel(); }
    else if (event.key === 'Enter' || event.code === 'NumpadEnter') { event.preventDefault(); confirm(); }
  };
  document.addEventListener('keydown', key, true);
  overlayRef.current?.focus({ preventScroll: true });
  return () => document.removeEventListener('keydown', key, true);
}, [cancel, confirm]);
```

Give the overlay `ref={overlayRef}`, `tabIndex={-1}`, and focus it on pointer down before selection starts. Keep one key listener, and keep arrow-key movement in the same handler.

- [ ] **Step 4: Run the focused capture component test**

Run: `pnpm exec vitest run src/features/capture/CaptureOverlay.test.tsx`

Expected: both Enter and Escape happy paths pass, including NumpadEnter normalization if included in the same parameterized positive test.

- [ ] **Step 5: Commit**

```text
git add src/features/capture/CaptureOverlay.tsx src/features/capture/CaptureOverlay.test.tsx src-tauri/src/commands/capture.rs
git commit -m "fix: make capture confirmation keys reliable"
```

### Task 4: Single compact top bar and anchored overlay menu

**Files:**
- Create: `src/app/AppTopBar.tsx`
- Create: `src/app/AppMenuOverlay.tsx`
- Create: `src/app/AppMenuOverlay.test.tsx`
- Modify: `src/app/App.tsx`
- Modify: `src/app/App.test.tsx`
- Modify: `src/styles.css`

**Interfaces:**
- Consumes: `activeView`, localized labels, `translationActive`, and `setActiveView` from `App`.
- Produces: `AppTopBarProps` and `AppMenuOverlayProps` with controlled `open`, `onClose`, `onNavigate`, and `locale` behavior.

- [ ] **Step 1: Add overlay interaction tests**

```tsx
it('opens from the hamburger and closes on Escape, outside click, and navigation', async () => {
  render(<App />);
  await user.click(screen.getByRole('button', { name: '메뉴 열기' }));
  expect(screen.getByRole('menu', { name: '앱 메뉴' })).toBeVisible();
  await user.keyboard('{Escape}');
  expect(screen.queryByRole('menu', { name: '앱 메뉴' })).not.toBeInTheDocument();
});
```

Also assert the page has one primary `tablist`, no `SmartCAT Translate` heading row, and no permanent account panel while the menu is closed.

- [ ] **Step 2: Run the focused App tests and confirm the old shell violates the new assertions**

Run: `pnpm exec vitest run src/app/App.test.tsx src/app/AppMenuOverlay.test.tsx`

Expected before implementation: new compact-shell assertions fail.

- [ ] **Step 3: Implement the controlled top bar and overlay**

```tsx
<AppTopBar
  activeView={activeView}
  menuOpen={menuOpen}
  onToggleMenu={() => setMenuOpen((open) => !open)}
  onNavigate={navigate}
/>
{menuOpen && (
  <AppMenuOverlay
    locale={accountLocale}
    onClose={() => setMenuOpen(false)}
    onNavigate={navigate}
  />
)}
```

`AppMenuOverlay` contains `AccountPanel`, settings/update/about/quit destinations, uses `role="menu"`, closes on document pointerdown outside and Escape, traps focus within the panel, and restores focus to the hamburger button via the controlled top-bar ref.

- [ ] **Step 4: Replace permanent rows with compact CSS**

Use `.app-shell-header { position: sticky; top: 0; }`, `.app-menu-overlay { position: absolute; inset-block-start: 100%; inset-inline-start: 0; width: min(21rem, calc(100vw - 1rem)); z-index: 50; }`, and an overflowable `.app-primary-tabs`. Do not reserve layout width or height for the closed overlay.

- [ ] **Step 5: Run the focused App tests**

Run: `pnpm exec vitest run src/app/App.test.tsx src/app/AppMenuOverlay.test.tsx`

Expected: one top navigation, transient overlay interactions, settings lock, recovery navigation, and existing view switching pass.

- [ ] **Step 6: Commit**

```text
git add src/app src/styles.css
git commit -m "feat: add compact overlay app shell"
```

### Task 5: Remove Text duplication and compact Settings

**Files:**
- Modify: `src/features/translation/TextWorkspace.tsx`
- Modify: `src/features/translation/TextWorkspace.test.tsx`
- Create: `src/features/settings/SettingsCategories.tsx`
- Modify: `src/features/settings/SettingsView.tsx`
- Modify: `src/features/settings/SettingsView.test.tsx`
- Modify: `src/styles.css`

**Interfaces:**
- Consumes: existing `AppSettings`, profile, glossary, hotkey, model, update, and lifecycle operations.
- Produces: `SettingsCategory = 'general' | 'translation' | 'shortcuts' | 'privacy' | 'updates'` and a controlled `SettingsCategories` rail.

- [ ] **Step 1: Update focused UI tests**

```tsx
expect(screen.queryByRole('tablist', { name: '번역 작업 공간' })).not.toBeInTheDocument();
expect(screen.getByRole('tablist', { name: '설정 분류' })).toBeVisible();
await user.click(screen.getByRole('tab', { name: '단축키' }));
expect(screen.getByText('접근성 및 키보드 권한')).toBeVisible();
```

Keep existing persistence, model refresh, lifecycle, and profile tests; change queries only where the approved structure intentionally moved controls.

- [ ] **Step 2: Run the two focused test files**

Run: `pnpm exec vitest run src/features/translation/TextWorkspace.test.tsx src/features/settings/SettingsView.test.tsx`

Expected before implementation: the no-duplicate-tabs and category assertions fail.

- [ ] **Step 3: Remove Text workspace navigation placeholders**

Delete `.workspace-tabs`, hidden placeholder tabpanels, and the permanent `.workspace-meta` row. Keep the language bar, connected-state information only when it is actionable, translation panes, actions, and `aria-live` status.

- [ ] **Step 4: Add compact Settings categories without changing persistence**

```ts
export type SettingsCategory = 'general' | 'translation' | 'shortcuts' | 'privacy' | 'updates';
```

General owns locale/theme/lifecycle; Translation owns profile/glossary/model; Shortcuts owns `HotkeySettings`; Privacy owns history retention and close behavior; Updates owns `UpdatePanel`. Render only the active category, but keep `settings` as the single draft object and the existing save/update commands as the only persistence paths.

- [ ] **Step 5: Add responsive dense layout CSS**

Use a `10.5rem minmax(0, 1fr)` settings grid above 760px, a single column below 760px, two-column field grids where labels fit, `0.65rem` section gaps, and no fixed empty heights. The category rail becomes a horizontal scroll row at the minimum viewport.

- [ ] **Step 6: Run focused UI tests**

Run: `pnpm exec vitest run src/features/translation/TextWorkspace.test.tsx src/features/settings/SettingsView.test.tsx src/app/App.test.tsx src/app/AppMenuOverlay.test.tsx`

Expected: all focused shell, translation, and settings tests pass.

- [ ] **Step 7: Commit**

```text
git add src/features/translation src/features/settings src/styles.css
git commit -m "feat: compact translation and settings workspaces"
```

### Task 6: Records, production package, installation, and happy-path smoke

**Files:**
- Modify: `PROJECT_LOG.txt`
- Modify: `DECISIONS.txt` only if implementation changes an approved decision.
- Modify: `CHANGELOG.md`
- Generate: `dist-installers/windows/0.1.0/*`

**Interfaces:**
- Consumes: completed Tasks 1–5 and the existing pinned `smartcat-codex` runtime.
- Produces: clean committed source, refreshed MSI/NSIS installers, installed app smoke evidence, hashes, and an updated screenshot.

- [ ] **Step 1: Run format and focused frontend gates**

Run: `pnpm build`

Run: `pnpm exec vitest run src/app/App.test.tsx src/app/AppMenuOverlay.test.tsx src/features/account/AccountPanel.test.tsx src/features/capture/CaptureOverlay.test.tsx src/features/translation/TextWorkspace.test.tsx src/features/settings/SettingsView.test.tsx`

Expected: TypeScript/Vite build and selected happy-path tests pass.

- [ ] **Step 2: Run one Rust gate at a time**

Before each command, confirm cargo/rustc/link process count is zero.

Run: `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test codex_auth`

Run: `cargo check --manifest-path src-tauri/Cargo.toml --bin smartcat-translate`

Expected: formatting, focused auth behavior, and desktop compilation pass.

- [ ] **Step 3: Record implementation and verification**

Append content-free entries describing exact changed behavior and command outcomes. Add a `0.1.0` prerelease changelog section covering console suppression, compact shell, login opener, and capture keys.

- [ ] **Step 4: Build one Windows release package**

Run the existing `pnpm release:package:windows` path with the already-built pinned runtime. Do not rebuild the Codex runtime unless the asset verifier reports it missing or mismatched.

Expected: fresh MSI and NSIS installers under `D:\SmartCATTranslate\dist-installers\windows\0.1.0`.

- [ ] **Step 5: Install the NSIS package and run short manual smoke**

Verify: no terminal window; main window cannot resize below 760×520; hamburger overlay opens/closes without moving content; ChatGPT login launches the default browser; capture Enter confirms and Escape cancels; installed sidecar returns `codex-cli 0.144.4`.

- [ ] **Step 6: Capture evidence and hashes**

Save a current main-window screenshot in the installer evidence folder. Compute SHA-256 for MSI and NSIS and record both without logging credentials, source text, auth URLs, or document content.

- [ ] **Step 7: Run records/privacy checks and commit**

Run: `pnpm records:check`

Run: `pnpm privacy:check`

Run: `git diff --check`

```text
git add PROJECT_LOG.txt DECISIONS.txt CHANGELOG.md
git commit -m "chore: record compact shell release verification"
```

Expected: clean worktree and a final report that distinguishes locally verified Windows behavior from still-unbuilt macOS artifacts and unsigned public release work.
