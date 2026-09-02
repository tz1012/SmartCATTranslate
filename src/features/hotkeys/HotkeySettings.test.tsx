import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { HotkeySettings } from './HotkeySettings';
import type { ConflictReport } from './hotkeyApi';

const invoke = vi.fn();

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invoke(...args),
}));

afterEach(() => {
  cleanup();
  invoke.mockReset();
});

describe('HotkeySettings', () => {
  it('ignores a previous conflict result after a new recording starts', async () => {
    let finishAnalysis!: (report: ConflictReport) => void;
    const pendingAnalysis = new Promise<ConflictReport>((resolve) => {
      finishAnalysis = resolve;
    });
    invoke.mockImplementation((command: string) => {
      if (command === 'list_hotkeys' || command === 'list_blocked_apps') return Promise.resolve([]);
      if (command === 'suspend_hotkeys') return Promise.resolve();
      if (command === 'analyze_hotkey') return pendingAnalysis;
      throw new Error(`unexpected command: ${command}`);
    });
    render(<HotkeySettings locale="ko" defaultProfileId="profile-1" />);

    fireEvent.click(await screen.findByRole('button', { name: '새 단축키 녹화' }));
    const recorder = screen.getByRole('group', { name: '단축키 녹화' });
    fireEvent.keyDown(recorder, {
      key: 't',
      code: 'KeyT',
      ctrlKey: true,
      altKey: true,
      shiftKey: true,
    });
    fireEvent.click(screen.getByRole('button', { name: '완료' }));
    fireEvent.click(screen.getByRole('button', { name: '새 단축키 녹화' }));

    await act(async () => {
      finishAnalysis({
        level: 'confirmed',
        causes: [{
          severity: 'blocking',
          description: '단축키 충돌을 확인하는 중 오류가 발생했습니다.',
          application: null,
          feature: null,
          sourceUrl: null,
          verifiedAt: null,
        }],
        alternatives: [],
        canForce: false,
      });
    });

    expect(screen.queryByText('단축키 충돌을 확인하는 중 오류가 발생했습니다.')).not.toBeInTheDocument();
  });
});
