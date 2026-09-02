import { cleanup, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { invoke } from '@tauri-apps/api/core';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => undefined) }));

import { UpdatePanel } from './UpdatePanel';

describe('UpdatePanel manual releases', () => {
  beforeEach(() => {
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'get_update_recovery_instructions') return null;
      if (command === 'check_for_update') return {
        available: true,
        version: '0.1.4',
        releaseNotes: 'Shortcut fixes',
        publishedAt: '2026-09-02T00:00:00Z',
        manualOnly: true,
        releaseUrl: 'https://github.com/tz1012/SmartCATTranslate/releases/tag/app-v0.1.4',
      };
      if (command === 'open_update_release') return undefined;
      throw new Error(`unexpected command: ${command}`);
    });
  });

  afterEach(() => {
    cleanup();
    vi.clearAllMocks();
  });

  it('opens a public release page without preparing or installing an update', async () => {
    render(<UpdatePanel locale="en" />);

    await userEvent.click(screen.getByRole('button', { name: 'Check for updates' }));
    await userEvent.click(await screen.findByRole('button', { name: 'Open release page' }));

    expect(invoke).toHaveBeenCalledWith('open_update_release', {
      url: 'https://github.com/tz1012/SmartCATTranslate/releases/tag/app-v0.1.4',
    });
    expect(invoke).not.toHaveBeenCalledWith('prepare_update', expect.anything());
    expect(invoke).not.toHaveBeenCalledWith('install_update', expect.anything());
  });
});
