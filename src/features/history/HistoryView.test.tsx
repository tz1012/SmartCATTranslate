import { cleanup, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { invoke } from '@tauri-apps/api/core';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { HistoryView } from './HistoryView';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe('HistoryView', () => {
  it('shows readable records with a warning when damaged records were skipped', async () => {
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'get_settings') return {
        schemaVersion: 2, locale: 'ko', theme: 'light', defaultProfileId: null, profiles: [], glossary: [], selectedModel: { type: 'automatic' }, launchAtLogin: false, closeBehavior: 'keepInTray', quickAccessPosition: 'popup', historyRetentionDays: 30,
      };
      if (command === 'list_history') return {
        records: [{ id: 'record-1', createdAt: '2026-09-03T00:00:00Z', kind: 'text', sourceLanguage: 'en', targetLanguage: 'ko', source: 'Hello', result: '안녕하세요', displayName: null, warningCount: 0 }],
        nextCursor: null,
        unreadableCount: 1,
      };
      throw new Error(`unexpected command: ${command}`);
    });

    render(<HistoryView locale="ko" />);

    expect(await screen.findByText('Hello')).toBeVisible();
    expect(screen.getByText('손상되어 열 수 없는 기록 1건을 건너뛰었습니다.')).toBeVisible();
    expect(screen.queryByText('기록을 열 수 없습니다.')).not.toBeInTheDocument();
  });

  it('lets the user retry a failed history load and displays the recovered records', async () => {
    let reads = 0;
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'get_settings') return {
        schemaVersion: 2, locale: 'ko', theme: 'light', defaultProfileId: null, profiles: [], glossary: [], selectedModel: { type: 'automatic' }, launchAtLogin: false, closeBehavior: 'keepInTray', quickAccessPosition: 'popup', historyRetentionDays: 30,
      };
      if (command === 'list_history') {
        reads += 1;
        if (reads === 1) throw new Error('private database failure');
        return { records: [{ id: 'record-1', createdAt: '2026-09-03T00:00:00Z', kind: 'text', sourceLanguage: 'en', targetLanguage: 'ko', source: 'Hello', result: '안녕하세요', displayName: null, warningCount: 0 }], nextCursor: null };
      }
      throw new Error(`unexpected command: ${command}`);
    });
    render(<HistoryView locale="ko" />);

    expect(await screen.findByRole('alert')).toHaveTextContent('기록을 열 수 없습니다.');
    expect(screen.queryByText('private database failure')).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: '다시 시도' }));

    await waitFor(() => expect(screen.getByText('Hello')).toBeVisible());
    expect(screen.getByLabelText('번역문')).toHaveValue('안녕하세요');
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });
});
