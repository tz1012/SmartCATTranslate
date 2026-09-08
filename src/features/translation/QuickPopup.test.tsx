import { cleanup, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { emitTo } from '@tauri-apps/api/event';

import { QuickPopup } from './QuickPopup';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({
  emitTo: vi.fn(),
  listen: vi.fn(async (_event: string, handler: (event: { payload: unknown }) => void) => {
    queueMicrotask(() => handler({
      payload: {
        requestId: 'request-1',
        request: {
          text: 'Find your next growth opportunity.',
          profile: { sourceLanguage: null, targetLanguage: 'ko', quality: 'balanced', tone: 'natural', protectedTerms: [] },
          field: 'general',
          glossary: [],
          mode: 'translate',
          secret: false,
        },
        profileName: 'Default',
        locale: 'en',
        error: 'no_selection',
      },
    }));
    return vi.fn();
  }),
}));
vi.mock('../history/secretMode', () => ({ useSecretMode: () => [false, vi.fn()] }));
vi.mock('../history/historyApi', () => ({ saveHistoryRecord: vi.fn(async () => 'record-1') }));
vi.mock('./useTranslationJob', () => ({
  useTranslationJob: () => ({
    state: { status: 'completed', text: '다음 성장 기회를 찾아보세요.', jobId: 'job-1' },
    listenerState: 'ready',
    start: vi.fn(),
    cancel: vi.fn(),
    reset: vi.fn(),
    retryListener: vi.fn(),
  }),
}));

afterEach(cleanup);

describe('QuickPopup branding', () => {
  it('uses the BYOK Translator name in its accessible label and header', async () => {
    render(<QuickPopup />);

    expect(await screen.findByRole('region', { name: 'BYOK Translator quick translation' })).toBeVisible();
    expect(screen.getByText('BYOK Translator')).toBeVisible();
  });

  it('hands the completed translation to the main window and closes the popup', async () => {
    render(<QuickPopup />);

    await userEvent.click(await screen.findByRole('button', { name: 'Open in main window' }));

    expect(emitTo).toHaveBeenCalledWith('main', 'quick-popup-open-main', {
      id: 'request-1',
      source: 'Find your next growth opportunity.',
      translation: '다음 성장 기회를 찾아보세요.',
    });
    expect(invoke).toHaveBeenCalledWith('open_main_window');
    expect(invoke).toHaveBeenCalledWith('close_quick_popup');
  });
});
