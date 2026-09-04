import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { QuickPopup } from './QuickPopup';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (_event: string, handler: (event: { payload: unknown }) => void) => {
    queueMicrotask(() => handler({
      payload: {
        requestId: 'request-1',
        request: null,
        profileName: 'Default',
        locale: 'en',
        error: 'no_selection',
      },
    }));
    return vi.fn();
  }),
}));
vi.mock('../history/secretMode', () => ({ useSecretMode: () => [false, vi.fn()] }));
vi.mock('../history/historyApi', () => ({ saveHistoryRecord: vi.fn() }));
vi.mock('./useTranslationJob', () => ({
  useTranslationJob: () => ({
    state: { status: 'idle', text: '' },
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
});
