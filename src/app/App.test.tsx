import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { App } from './App';

type EventHandler = (event: { payload: unknown }) => void;
const eventBridge = vi.hoisted(() => ({ translation: null as EventHandler | null }));

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (name: string, handler: EventHandler) => {
    if (name === 'translation-event') eventBridge.translation = handler;
    return () => undefined;
  }),
}));

afterEach(() => {
  cleanup();
  eventBridge.translation = null;
  vi.clearAllMocks();
});

describe('App', () => {
  it('mounts the complete text translation workspace', async () => {
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'get_settings') return {
        schemaVersion: 1, locale: 'ko', theme: 'system', defaultProfileId: 'default-profile',
        profiles: [{ id: 'default-profile', name: '기본 프로필', field: 'general', profile: { sourceLanguage: null, targetLanguage: 'ko', quality: 'balanced', tone: 'natural', protectedTerms: [] } }],
        glossary: [], selectedModel: { type: 'automatic' }, launchAtLogin: false, closeBehavior: 'keepInTray', quickAccessPosition: 'popup', historyRetentionDays: 30,
      };
      if (command === 'get_account') return { account: { state: 'signedOut' }, loginPending: false };
      throw new Error(`unexpected command: ${command}`);
    });
    render(<App />);
    expect(screen.getByRole('heading', { name: 'SmartCAT Translate' })).toBeVisible();
    expect(await screen.findByLabelText('원문')).toBeVisible();
    expect(screen.getByLabelText('번역문')).toBeVisible();
    expect(screen.getByRole('tab', { name: '텍스트' })).toHaveAttribute('aria-selected', 'true');
  });

  it('shows the text translation workspace in English', async () => {
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'get_settings') return {
        schemaVersion: 1, locale: 'en', theme: 'system', defaultProfileId: 'default-profile',
        profiles: [{ id: 'default-profile', name: 'Default profile', field: 'general', profile: { sourceLanguage: null, targetLanguage: 'ko', quality: 'balanced', tone: 'natural', protectedTerms: [] } }],
        glossary: [], selectedModel: { type: 'automatic' }, launchAtLogin: false, closeBehavior: 'keepInTray', quickAccessPosition: 'popup', historyRetentionDays: 30,
      };
      if (command === 'get_account') return { account: { state: 'signedOut' }, loginPending: false };
      throw new Error(`unexpected command: ${command}`);
    });
    const { container } = render(<App locale="en" />);
    expect(await screen.findByRole('region', { name: 'Text translation' })).toBeVisible();
    expect(screen.getByLabelText('Source text')).toBeVisible();
    expect(screen.getByLabelText('Translation')).toBeVisible();
    expect(screen.getByRole('complementary', { name: 'ChatGPT account' })).toBeVisible();
    expect(container.textContent).not.toMatch(/[\u3131-\u318E\uAC00-\uD7A3]/u);
    for (const element of container.querySelectorAll('[aria-label]')) {
      expect(element.getAttribute('aria-label')).not.toMatch(/[\u3131-\u318E\uAC00-\uD7A3]/u);
    }
  });

  it('uses the saved interface locale when no locale override is provided', async () => {
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'get_settings') return {
        schemaVersion: 1, locale: 'en', theme: 'system', defaultProfileId: 'default-profile',
        profiles: [{ id: 'default-profile', name: 'Default profile', field: 'general', profile: { sourceLanguage: null, targetLanguage: 'ko', quality: 'balanced', tone: 'natural', protectedTerms: [] } }],
        glossary: [], selectedModel: { type: 'automatic' }, launchAtLogin: false, closeBehavior: 'keepInTray', quickAccessPosition: 'popup', historyRetentionDays: 30,
      };
      if (command === 'get_account') return { account: { state: 'signedOut' }, loginPending: false };
      throw new Error(`unexpected command: ${command}`);
    });

    render(<App />);

    expect(await screen.findByRole('region', { name: 'Text translation' })).toBeVisible();
    expect(screen.getByRole('complementary', { name: 'ChatGPT account' })).toBeVisible();
  });

  it('mounts the settings screen through nontechnical accessible navigation', async () => {
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'get_settings') return {
        schemaVersion: 1, locale: 'ko', theme: 'dark', defaultProfileId: 'default-profile',
        profiles: [{ id: 'default-profile', name: '기본 프로필', field: 'general', profile: { sourceLanguage: null, targetLanguage: 'ko', quality: 'balanced', tone: 'natural', protectedTerms: [] } }],
        glossary: [], selectedModel: { type: 'automatic' }, launchAtLogin: false, closeBehavior: 'keepInTray', quickAccessPosition: 'popup', historyRetentionDays: 30,
      };
      if (command === 'get_account') return { account: { state: 'signedOut' }, loginPending: false };
      if (command === 'get_rate_limits') return { primaryUsedPercent: null, primaryResetsAt: null, secondaryUsedPercent: null, secondaryResetsAt: null };
      if (command === 'list_available_models') return [];
      throw new Error(`unexpected command: ${command}`);
    });
    const { container } = render(<App />);
    const translateTab = await screen.findByRole('tab', { name: '번역' });
    const settingsTab = screen.getByRole('tab', { name: '설정' });
    expect(translateTab).toHaveAttribute('aria-controls', 'app-panel-translate');
    expect(settingsTab).toHaveAttribute('aria-controls', 'app-panel-settings');
    expect(container.querySelector('main')).toHaveAttribute('data-theme', 'dark');

    await userEvent.click(settingsTab);
    expect(screen.getByRole('tabpanel', { name: '설정' })).toBeVisible();
    expect(screen.getByRole('heading', { name: '설정' })).toBeVisible();
    expect(screen.getByLabelText('번역 프로필')).toBeVisible();
  });

  it('keeps the translation workspace mounted and locks settings navigation until the active job terminates', async () => {
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'get_settings') return {
        schemaVersion: 1, locale: 'ko', theme: 'system', defaultProfileId: 'default-profile',
        profiles: [{ id: 'default-profile', name: '기본 프로필', field: 'general', profile: { sourceLanguage: null, targetLanguage: 'ko', quality: 'balanced', tone: 'natural', protectedTerms: [] } }],
        glossary: [], selectedModel: { type: 'automatic' }, launchAtLogin: false, closeBehavior: 'keepInTray', quickAccessPosition: 'popup', historyRetentionDays: 30,
      };
      if (command === 'get_account') return { account: { state: 'signedIn' }, loginPending: false };
      if (command === 'translate_text') return 'job-1';
      if (command === 'get_rate_limits') return { primaryUsedPercent: null, primaryResetsAt: null, secondaryUsedPercent: null, secondaryResetsAt: null };
      if (command === 'list_available_models') return [];
      throw new Error(`unexpected command: ${command}`);
    });
    const user = userEvent.setup();
    render(<App />);
    const source = await screen.findByLabelText('원문');
    await user.type(source, 'Hello');
    await user.click(screen.getByRole('button', { name: '번역' }));

    const settingsTab = screen.getByRole('tab', { name: '설정' });
    await waitFor(() => expect(settingsTab).toBeDisabled());
    expect(screen.getByText('번역 또는 취소 처리 중에는 설정을 열 수 없습니다.')).toBeVisible();
    fireEvent.click(settingsTab);
    fireEvent.keyDown(settingsTab, { key: 'Enter' });
    expect(source).toBeInTheDocument();
    expect(screen.queryByRole('heading', { name: '설정' })).not.toBeInTheDocument();

    act(() => eventBridge.translation?.({ payload: {
      type: 'completed', jobId: 'job-1', result: { translatedText: '안녕하세요', detectedLanguage: 'en' },
    } }));
    await waitFor(() => expect(settingsTab).toBeEnabled());
    await user.click(settingsTab);
    expect(await screen.findByRole('heading', { name: '설정' })).toBeVisible();
  });

  it('explains the active translation navigation lock in English', async () => {
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'get_settings') return {
        schemaVersion: 1, locale: 'en', theme: 'system', defaultProfileId: 'default-profile',
        profiles: [{ id: 'default-profile', name: 'Default profile', field: 'general', profile: { sourceLanguage: null, targetLanguage: 'ko', quality: 'balanced', tone: 'natural', protectedTerms: [] } }],
        glossary: [], selectedModel: { type: 'automatic' }, launchAtLogin: false, closeBehavior: 'keepInTray', quickAccessPosition: 'popup', historyRetentionDays: 30,
      };
      if (command === 'get_account') return { account: { state: 'signedIn' }, loginPending: false };
      if (command === 'translate_text') return 'job-1';
      throw new Error(`unexpected command: ${command}`);
    });
    const user = userEvent.setup();
    render(<App locale="en" />);
    await user.type(await screen.findByLabelText('Source text'), 'Hello');
    await user.click(screen.getByRole('button', { name: 'Translate' }));

    expect(await screen.findByText('Settings are unavailable while translation or cancellation is in progress.')).toBeVisible();
    expect(screen.getByRole('tab', { name: 'Settings' })).toBeDisabled();
  });
});
