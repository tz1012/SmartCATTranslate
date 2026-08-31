# SmartCAT Translate compact shell and runtime fixes design

Date: 2026-08-31
Status: approved direction; pending implementation

## Objective

Make the installed desktop app behave like a compact translation tool: no console window, one DeepL-inspired navigation bar, space-efficient settings, working ChatGPT browser sign-in, and reliable Enter/Escape controls during screen capture.

## Scope and success criteria

- A release Windows build starts as a GUI process without opening or attaching a console window. Content-free diagnostics remain available through redirected logs and development builds.
- The main window uses one compact top bar containing a hamburger button, Text, Image & screen, Documents, History, and a compact account control. The separate product-title row, permanent account banner, duplicate Text workspace tabs, and permanent metadata row are removed.
- The hamburger button opens an anchored overlay menu above the current content; it does not push or resize the workspace. The overlay owns account details, Settings, update access, help/about information, and quit. Temporary warnings remain contextual and do not reserve an empty row.
- Settings use a compact category rail and dense content panel. Categories are General, Translation, Shortcuts, Privacy & history, and Updates. Existing controls and data behavior remain intact.
- ChatGPT sign-in opens only the validated official `https://chatgpt.com` host family in the user's default browser and keeps the existing managed Codex account flow. Failure messages remain sanitized.
- Screen-capture Enter confirms a valid selection and Escape cancels from any capture overlay, including multi-monitor sessions. Pointer selection and arrow-key adjustment continue to work.

## UI architecture

`App` remains the single view owner. A new compact shell splits presentation into three focused components:

1. `AppTopBar` renders primary modes and the hamburger/account controls.
2. `AppMenuOverlay` renders account state and secondary commands as a floating panel anchored below the hamburger button without consuming permanent space.
3. `SettingsView` keeps settings persistence but adds an internal category selection and compact grouped layout.

The Text workspace will render only the language controls, translation panes, actions, and contextual status. Its duplicate navigation tabs and hidden placeholder panels are removed. Account status becomes a small top-bar indicator; detailed status and login actions live in the menu overlay. The overlay closes on outside click, Escape, selecting a destination, or toggling the hamburger button, and always starts closed on a new app launch. Privacy and recovery warnings continue to appear above the active work area only while relevant.

At narrow widths, the primary mode bar scrolls horizontally and the overlay uses the available viewport width without changing the workspace layout. All controls keep semantic buttons, tab selection state, accessible names, focus visibility, focus containment while open, focus restoration to the hamburger button, and Escape-to-close behavior.

The main window opens at 1000×700, remains freely resizable without an enforced aspect ratio, and has a fixed minimum size of 760×520. Responsive layout handles the minimum viewport: primary modes can scroll horizontally, settings collapse to one content column, and the menu overlay clamps to the available window bounds. The fixed quick popup and monitor-sized capture overlays retain their separate sizing policies.

## Runtime fixes

### Windows console

The desktop entry point will opt into the Windows GUI subsystem for non-debug builds. Debug builds retain console output for development. This addresses the executable subsystem at its source rather than suppressing individual diagnostic messages.

### ChatGPT browser sign-in

The account command will use the initialized Tauri opener through the application handle, matching the already-used document/update opening path. URL validation remains before OS handoff, login cancellation remains authoritative after an opener failure, and no auth URL or login identifier is logged. The command boundary will distinguish browser-open failure from account-service/bootstrap failure without exposing sensitive detail.

### Capture keyboard handling

Each overlay will explicitly take focus when interacted with and its web handler will accept standard Enter, numpad Enter, Escape, and legacy key representations. The backend capture session will also receive a minimal native confirmation/cancellation path so behavior does not depend solely on which monitor webview currently owns focus. Completion and cancellation remain idempotent and close all overlay windows.

## Data and safety boundaries

- Official ChatGPT URL validation, managed login cancellation, sanitized errors, clipboard preservation, blocked-app checks, private temporary storage, and secret-free diagnostics are unchanged.
- No user document, history, credential, or existing test is removed.
- `sources/` remains read-only. Source, build tools, records, and installers remain on drive D where possible.

## Verification

Speed remains the priority. No new failure-injection suite or long full regression is added. Verification consists of focused component tests for the changed UI/key behavior, Rust formatting and focused compile/tests for entry point/opener/capture changes, one production package build, and short installed-app smoke checks:

1. launch without a console window;
2. compact navigation/menu-overlay/settings visual inspection against the supplied DeepL references;
3. ChatGPT login opens the default browser;
4. capture selection confirms with Enter and cancels with Escape;
5. installer and installed sidecar still launch.

Every decision, changed file, test result, remaining limitation, installer hash, and commit is recorded in `PROJECT_LOG.txt`, `DECISIONS.txt`, and `CHANGELOG.md`.
