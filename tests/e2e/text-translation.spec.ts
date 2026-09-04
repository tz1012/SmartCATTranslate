import { expect, test } from '@playwright/test';

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    const callbacks = new Map<number, (data: unknown) => void>();
    const listeners = new Map<string, number[]>();
    let callbackId = 0;
    const browserWindow = window as Window & {
      __TAURI_INTERNALS__: Record<string, unknown>;
      __TAURI_EVENT_PLUGIN_INTERNALS__: Record<string, unknown>;
      __copied?: string;
      __translationRequest?: unknown;
      __holdTranslation?: boolean;
    };
    const settings = {
      schemaVersion: 1,
      locale: 'ko',
      theme: 'light',
      defaultProfileId: 'default-profile',
      profiles: [{
        id: 'default-profile',
        name: '기본 프로필',
        field: 'general',
        profile: { sourceLanguage: null, targetLanguage: 'ko', quality: 'balanced', tone: 'natural', protectedTerms: [] },
      }],
      glossary: [],
      selectedModel: { type: 'automatic' },
      launchAtLogin: false,
      closeBehavior: 'keepInTray',
      quickAccessPosition: 'popup',
      historyRetentionDays: 30,
    };
    const emit = (event: string, payload: unknown) => {
      for (const id of listeners.get(event) ?? []) {
        callbacks.get(id)?.({ event, id, payload });
      }
    };
    browserWindow.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
      unregisterListener: (event: string, id: number) => {
        listeners.set(event, (listeners.get(event) ?? []).filter((candidate) => candidate !== id));
      },
    };
    browserWindow.__TAURI_INTERNALS__ = {
      metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main' } },
      transformCallback: (callback: (data: unknown) => void) => {
        callbackId += 1;
        callbacks.set(callbackId, callback);
        return callbackId;
      },
      unregisterCallback: (id: number) => callbacks.delete(id),
      invoke: async (command: string, args?: Record<string, unknown>) => {
        if (command === 'plugin:event|listen') {
          const event = String(args?.event);
          const handler = Number(args?.handler);
          listeners.set(event, [...(listeners.get(event) ?? []), handler]);
          return handler;
        }
        if (command === 'plugin:event|unlisten') return null;
        if (command === 'plugin:app|version') return '0.1.6';
        if (command === 'get_settings') return structuredClone(settings);
        if (command === 'get_account') return { account: { state: 'signedIn', emailHint: 'an***@example.com', plan: 'Plus' }, loginPending: false };
        if (command === 'get_lifecycle_status') return { launchAtLoginAvailable: true, launchAtLoginEnabled: false, hotkeysPaused: false };
        if (command === 'get_privacy_status') return { cleanupPending: false, retentionPending: false };
        if (command === 'check_for_update') return { available: false, version: null };
        if (command === 'list_available_models' || command === 'list_hotkeys' || command === 'list_blocked_apps' || command === 'list_history' || command === 'list_recoverable_jobs') return [];
        if (command === 'get_rate_limits') return { primaryUsedPercent: null, primaryResetsAt: null, secondaryUsedPercent: null, secondaryResetsAt: null };
        if (command === 'translate_text') {
          browserWindow.__translationRequest = args?.request;
          if (browserWindow.__holdTranslation) return 'e2e-job';
          setTimeout(() => {
            emit('translation-event', { type: 'delta', jobId: 'e2e-job', text: '서식을 ' });
            emit('translation-event', { type: 'delta', jobId: 'e2e-job', text: '유지하세요' });
            emit('translation-event', {
              type: 'completed',
              jobId: 'e2e-job',
              result: { translatedText: '서식을 유지하세요', detectedLanguage: 'en' },
            });
          }, 25);
          return 'e2e-job';
        }
        if (command === 'cancel_translation') return true;
        if (command === 'save_history_record') return 'e2e-history-record';
        throw new Error(`Unexpected command: ${command}`);
      },
    };
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: { writeText: async (text: string) => { browserWindow.__copied = text; } },
    });
  });
});

test('sends the exact request, streams, copies and lays out the panes responsively', async ({ page }, testInfo) => {
  await page.goto('/');
  await page.getByRole('textbox', { name: '원문', exact: true }).fill('Keep formatting');
  await page.getByRole('button', { name: '번역', exact: true }).click();

  await expect.poll(() => page.evaluate(() => (window as Window & { __translationRequest?: unknown }).__translationRequest)).toEqual({
    text: 'Keep formatting',
    profile: { sourceLanguage: null, targetLanguage: 'ko', quality: 'balanced', tone: 'natural', protectedTerms: [] },
    field: 'general',
    glossary: [],
    mode: 'translate',
    secret: false,
  });

  await expect(page.getByRole('textbox', { name: '번역문', exact: true })).toHaveValue('서식을 유지하세요');
  await page.getByRole('button', { name: '번역문 복사' }).click();
  await expect.poll(() => page.evaluate(() => (window as Window & { __copied?: string }).__copied)).toBe('서식을 유지하세요');

  const source = await page.locator('.source-pane').boundingBox();
  const result = await page.locator('.result-pane').boundingBox();
  expect(source).not.toBeNull();
  expect(result).not.toBeNull();
  if (testInfo.project.name === 'mobile-390x844') {
    expect(result!.y).toBeGreaterThan(source!.y + source!.height - 2);
    expect(Math.abs(result!.x - source!.x)).toBeLessThan(2);
  } else {
    expect(result!.x).toBeGreaterThan(source!.x + source!.width - 2);
    expect(Math.abs(result!.y - source!.y)).toBeLessThan(2);
  }
  await expect(page.locator('main')).toHaveAttribute('data-theme', 'light');

  await page.getByRole('button', { name: '메뉴 열기' }).click();
  await page.getByRole('button', { name: '일반 설정' }).click();
  await expect(page.getByRole('heading', { name: '설정' })).toBeVisible();
  await expect(page.locator('#app-panel-translate')).toBeHidden();

  await page.keyboard.press('Escape');
  await expect(page.getByRole('heading', { name: '설정' })).toHaveCount(0);
  await expect(page.locator('#app-panel-translate')).toBeVisible();
});

test('keeps the character count above the footer in a compact window', async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-1100x760', 'desktop minimum-window regression');
  await page.setViewportSize({ width: 760, height: 520 });
  await page.goto('/');

  const counter = page.locator('.source-pane small');
  const footer = page.locator('.workspace-footer');
  await expect(counter).toBeVisible();
  const counterBox = await counter.boundingBox();
  const footerBox = await footer.boundingBox();
  expect(counterBox).not.toBeNull();
  expect(footerBox).not.toBeNull();
  expect(counterBox!.y + counterBox!.height).toBeLessThanOrEqual(footerBox!.y + 1);
});

test('keeps the idle notification button compact at the right edge', async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-1100x760', 'desktop minimum-window regression');
  await page.setViewportSize({ width: 760, height: 520 });
  await page.goto('/');

  const header = page.locator('.app-shell-header');
  const notification = page.getByRole('button', { name: '알림' });
  const [headerBox, notificationBox] = await Promise.all([
    header.boundingBox(),
    notification.boundingBox(),
  ]);

  expect(headerBox).not.toBeNull();
  expect(notificationBox).not.toBeNull();
  expect(notificationBox!.width).toBeLessThanOrEqual(60);
  expect(Math.abs(notificationBox!.x + notificationBox!.width - (headerBox!.x + headerBox!.width))).toBeLessThanOrEqual(1);
});

test('keeps the translation notice clear of History and reserves the right edge for notifications', async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-1100x760', 'desktop minimum-window regression');
  await page.setViewportSize({ width: 760, height: 520 });
  await page.goto('/');
  await page.evaluate(() => { (window as Window & { __holdTranslation?: boolean }).__holdTranslation = true; });
  await page.getByRole('textbox', { name: '원문', exact: true }).fill('Hello world');
  await page.getByRole('button', { name: '번역', exact: true }).click();

  const history = page.getByRole('tab', { name: '기록' });
  const status = page.getByText('번역 또는 취소 처리 중에는 설정을 열 수 없습니다.');
  await expect(history).toBeVisible();
  await expect(status).toBeVisible();
  const historyBox = await history.boundingBox();
  const statusBox = await status.boundingBox();
  expect(historyBox).not.toBeNull();
  expect(statusBox).not.toBeNull();
  expect(historyBox!.x + historyBox!.width).toBeLessThanOrEqual(statusBox!.x + 1);
  const statusMetrics = await status.evaluate((element) => {
    const style = getComputedStyle(element);
    return { height: element.getBoundingClientRect().height, lineHeight: Number.parseFloat(style.lineHeight), whiteSpace: style.whiteSpace };
  });
  expect(statusMetrics.whiteSpace).toBe('normal');
  expect(statusMetrics.height).toBeGreaterThan(statusMetrics.lineHeight * 1.5);

  await expect(page.getByRole('button', { name: '계정 메뉴' })).toHaveCount(0);
  const headerBox = await page.locator('.app-shell-header').boundingBox();
  const notificationBox = await page.getByRole('button', { name: /알림 \d+개/ }).boundingBox();
  expect(headerBox).not.toBeNull();
  expect(notificationBox).not.toBeNull();
  expect(Math.abs((notificationBox!.x + notificationBox!.width) - (headerBox!.x + headerBox!.width))).toBeLessThanOrEqual(1);
});
