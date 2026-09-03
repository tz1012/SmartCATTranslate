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
        if (command === 'get_settings') return structuredClone(settings);
        if (command === 'get_account') return { account: { state: 'signedIn', emailHint: 'an***@example.com', plan: 'Plus' }, loginPending: false };
        if (command === 'get_rate_limits') return { primaryUsedPercent: null, primaryResetsAt: null, secondaryUsedPercent: null, secondaryResetsAt: null };
        if (command === 'translate_text') {
          browserWindow.__translationRequest = args?.request;
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
});
