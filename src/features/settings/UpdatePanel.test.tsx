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

  it('downloads, verifies, and installs a signed update from one Update click', async () => {
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'get_update_recovery_instructions') return null;
      if (command === 'check_for_update') return {
        available: true,
        version: '0.1.5',
        releaseNotes: 'One-click updates',
        publishedAt: '2026-09-03T00:00:00Z',
        consentToken: 'check-token',
        manualOnly: false,
      };
      if (command === 'prepare_update') return { installToken: 'install-token', sizeBytes: 2048 };
      if (command === 'authorize_update_restart') return { restartConsentToken: 'restart-token' };
      if (command === 'install_update') return undefined;
      throw new Error(`unexpected command: ${command}`);
    });
    render(<UpdatePanel locale="en" />);

    await userEvent.click(screen.getByRole('button', { name: 'Check for updates' }));
    await userEvent.click(await screen.findByRole('button', { name: 'Update' }));

    expect(invoke).toHaveBeenCalledWith('prepare_update', {
      version: '0.1.5',
      consentToken: 'check-token',
    });
    expect(invoke).toHaveBeenCalledWith('authorize_update_restart', {
      version: '0.1.5',
      installToken: 'install-token',
    });
    expect(invoke).toHaveBeenCalledWith('install_update', {
      version: '0.1.5',
      installToken: 'install-token',
      restartConsentToken: 'restart-token',
    });

    const commands = vi.mocked(invoke).mock.calls.map(([command]) => command);
    expect(commands.indexOf('prepare_update')).toBeLessThan(commands.indexOf('authorize_update_restart'));
    expect(commands.indexOf('authorize_update_restart')).toBeLessThan(commands.indexOf('install_update'));
  });

  it('refreshes the one-time consent after a failed download so Update can retry', async () => {
    let checks = 0;
    let preparations = 0;
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'get_update_recovery_instructions') return null;
      if (command === 'check_for_update') {
        checks += 1;
        return { available: true, version: '0.1.5', consentToken: `check-token-${checks}`, manualOnly: false };
      }
      if (command === 'prepare_update') {
        preparations += 1;
        if (preparations === 1) throw 'update_network_error';
        return { installToken: 'install-token', sizeBytes: 2048 };
      }
      if (command === 'authorize_update_restart') return { restartConsentToken: 'restart-token' };
      if (command === 'install_update') return undefined;
      throw new Error(`unexpected command: ${command}`);
    });
    render(<UpdatePanel locale="en" />);

    await userEvent.click(screen.getByRole('button', { name: 'Check for updates' }));
    await userEvent.click(await screen.findByRole('button', { name: 'Update' }));
    await screen.findByText('업데이트 서버에 연결하지 못했습니다. 네트워크를 확인하고 다시 시도하세요.');
    await userEvent.click(screen.getByRole('button', { name: 'Update' }));

    expect(invoke).toHaveBeenCalledWith('prepare_update', { version: '0.1.5', consentToken: 'check-token-2' });
    expect(invoke).toHaveBeenCalledWith('install_update', expect.objectContaining({ version: '0.1.5' }));
  });

  it('does not expose an action for incomplete manual-only release metadata', async () => {
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'get_update_recovery_instructions') return null;
      if (command === 'check_for_update') return { available: true, version: '0.1.5', manualOnly: true };
      throw new Error(`unexpected command: ${command}`);
    });
    render(<UpdatePanel locale="en" />);

    await userEvent.click(screen.getByRole('button', { name: 'Check for updates' }));

    expect(screen.queryByRole('button', { name: 'Update' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Open release page' })).not.toBeInTheDocument();
    expect(screen.getByRole('status')).toHaveTextContent('This release cannot be opened safely.');
  });
});
