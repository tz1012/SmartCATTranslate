# Short installed-app smoke checklist

Record the artifact SHA-256, OS version, architecture, tester, and result. Use only disposable mock documents and the acceptance-script data root.

- [ ] ChatGPT login opens the official flow, cancellation returns safely, and no token appears in logs.
- [ ] Text translation succeeds and secret mode leaves no history row.
- [ ] A global hotkey opens the quick popup; pause and conflict guidance work.
- [ ] Screen/image capture requests platform permission and a small mock capture completes.
- [ ] One small DOCX/PPTX/XLSX/PDF fixture produces a new output without changing the source.
- [ ] Encrypted history opens and deletes a mock record.
- [ ] A paused mock document job offers Continue/Delete/Later and resumes from its checkpoint.
- [ ] Update check is manual; notes/date/size appear; Later downloads nothing; explicit consent downloads and verifies; declining restart does not install or restart.
- [ ] Previous-installer/last-known-good instructions are visible after an update attempt; rollback is never automatic.
- [ ] Logs, diagnostics, temp roots, and artifacts contain no canary text, credentials, or full user path.

Windows local unsigned smoke may show the expected SmartScreen/unknown-publisher warning. Both macOS Intel and Apple Silicon installed-app smoke runs are CI-required before promotion.
