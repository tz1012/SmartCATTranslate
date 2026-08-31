import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { invoke } from '@tauri-apps/api/core';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { App } from './App';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => undefined) }));

const settings = {
  schemaVersion: 1,
  locale: 'ko',
  theme: 'system',
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

beforeEach(() => {
  vi.mocked(invoke).mockImplementation(async (command) => {
    if (command === 'get_settings') return structuredClone(settings);
    if (command === 'get_account') return { account: { state: 'signedOut' }, loginPending: false };
    if (command === 'get_privacy_status') return { cleanupPending: false, retentionPending: false };
    if (command === 'get_lifecycle_status') return { launchAtLoginAvailable: true, launchAtLoginEnabled: false, hotkeysPaused: false };
    if (command === 'list_history') return { records: [], nextCursor: null };
    if (command === 'list_recoverable_jobs') return [];
    if (command === 'list_available_models') return [];
    if (command === 'list_hotkeys') return [];
    if (command === 'list_blocked_apps') return [];
    return undefined;
  });
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe('App menu overlay', () => {
  it('keeps account details hidden until the hamburger button opens the overlay', async () => {
    render(<App />);
    await screen.findByLabelText('원문');

    expect(screen.queryByRole('heading', { name: 'SmartCAT Translate' })).not.toBeInTheDocument();
    expect(screen.queryByRole('complementary', { name: 'ChatGPT 계정' })).not.toBeInTheDocument();

    await userEvent.click(screen.getByRole('button', { name: '메뉴 열기' }));

    expect(screen.getByRole('dialog', { name: '앱 메뉴' })).toBeVisible();
    expect(screen.getByRole('complementary', { name: 'ChatGPT 계정' })).toBeVisible();
  });

  it('closes the overlay with Escape and restores hamburger focus', async () => {
    render(<App />);
    const hamburger = await screen.findByRole('button', { name: '메뉴 열기' });
    await userEvent.click(hamburger);

    fireEvent.keyDown(document, { key: 'Escape' });

    expect(screen.queryByRole('dialog', { name: '앱 메뉴' })).not.toBeInTheDocument();
    expect(hamburger).toHaveFocus();
  });

  it('closes the overlay after navigating to settings', async () => {
    render(<App />);
    await userEvent.click(await screen.findByRole('button', { name: '메뉴 열기' }));

    await userEvent.click(screen.getByRole('button', { name: '일반 설정' }));

    expect(screen.queryByRole('dialog', { name: '앱 메뉴' })).not.toBeInTheDocument();
    expect(await screen.findByRole('heading', { name: '설정' })).toBeVisible();
  });
});
