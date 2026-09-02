import { act, cleanup, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { StrictMode } from 'react';
import { App } from './App';

type EventHandler = (event: { payload: unknown }) => void;
const eventBridge = vi.hoisted(() => ({ translation: null as EventHandler | null }));

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((resolvePromise) => { resolve = resolvePromise; });
  return { promise, resolve };
}

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
  it('starts light before settings load and preserves an explicit saved dark theme', async () => {
    let settingsReads = 0;
    const firstSettings = deferred<Record<string, unknown>>();
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'get_settings') {
        settingsReads += 1;
        if (settingsReads === 1) return firstSettings.promise;
        return {
          schemaVersion: 2, locale: 'en', theme: 'dark', defaultProfileId: 'default-profile',
          profiles: [{ id: 'default-profile', name: 'Default profile', field: 'general', profile: { sourceLanguage: null, targetLanguage: 'ko', quality: 'balanced', tone: 'natural', protectedTerms: [] } }],
          glossary: [], selectedModel: { type: 'automatic' }, launchAtLogin: false, closeBehavior: 'keepInTray', quickAccessPosition: 'popup', historyRetentionDays: 30,
        };
      }
      if (command === 'get_account') return { account: { state: 'signedOut' }, loginPending: false };
      if (command === 'get_privacy_status') return { cleanupPending: false, retentionPending: false };
      if (command === 'get_lifecycle_status') return { launchAtLoginAvailable: true, launchAtLoginEnabled: false, hotkeysPaused: false };
      if (command === 'list_history' || command === 'list_recoverable_jobs') return [];
      throw new Error(`unexpected command: ${command}`);
    });
    const { container } = render(<App locale="en" />);

    expect(container.querySelector('main')).toHaveAttribute('data-theme', 'light');
    firstSettings.resolve({
      schemaVersion: 2, locale: 'en', theme: 'dark', defaultProfileId: 'default-profile',
      profiles: [{ id: 'default-profile', name: 'Default profile', field: 'general', profile: { sourceLanguage: null, targetLanguage: 'ko', quality: 'balanced', tone: 'natural', protectedTerms: [] } }],
      glossary: [], selectedModel: { type: 'automatic' }, launchAtLogin: false, closeBehavior: 'keepInTray', quickAccessPosition: 'popup', historyRetentionDays: 30,
    });
    await waitFor(() => expect(container.querySelector('main')).toHaveAttribute('data-theme', 'dark'));
  });

  it('keeps privacy maintenance notices behind a top-bar notification button', async () => {
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'get_settings') return {
        schemaVersion: 1, locale: 'ko', theme: 'system', defaultProfileId: 'default-profile',
        profiles: [{ id: 'default-profile', name: '기본 프로필', field: 'general', profile: { sourceLanguage: null, targetLanguage: 'ko', quality: 'balanced', tone: 'natural', protectedTerms: [] } }],
        glossary: [], selectedModel: { type: 'automatic' }, launchAtLogin: false, closeBehavior: 'keepInTray', quickAccessPosition: 'popup', historyRetentionDays: 30,
      };
      if (command === 'get_account') return { account: { state: 'signedOut' }, loginPending: false };
      if (command === 'get_privacy_status') return { cleanupPending: true, retentionPending: false };
      if (command === 'get_lifecycle_status') return { launchAtLoginAvailable: true, launchAtLoginEnabled: false, hotkeysPaused: false };
      if (command === 'list_history' || command === 'list_recoverable_jobs') return [];
      if (command === 'mark_app_healthy') return true;
      throw new Error(`unexpected command: ${command}`);
    });
    render(<App />);

    const notice = '임시 파일 정리가 보류되었습니다. 앱이 시작될 때 안전하게 다시 시도합니다.';
    await screen.findByLabelText('원문');
    expect(screen.queryByText(notice)).not.toBeInTheDocument();
    const button = screen.getByRole('button', { name: '알림 1개' });
    expect(button).toHaveAttribute('aria-expanded', 'false');

    await userEvent.click(button);
    expect(screen.getByRole('dialog', { name: '알림' })).toHaveTextContent(notice);
  });

  it('checks once at startup and notifies without downloading an available update', async () => {
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'get_settings') return {
        schemaVersion: 2, locale: 'ko', theme: 'light', defaultProfileId: 'default-profile',
        profiles: [{ id: 'default-profile', name: '기본 프로필', field: 'general', profile: { sourceLanguage: null, targetLanguage: 'ko', quality: 'balanced', tone: 'natural', protectedTerms: [] } }],
        glossary: [], selectedModel: { type: 'automatic' }, launchAtLogin: false, closeBehavior: 'keepInTray', quickAccessPosition: 'popup', historyRetentionDays: 30,
      };
      if (command === 'get_account') return { account: { state: 'signedOut' }, loginPending: false };
      if (command === 'get_privacy_status') return { cleanupPending: false, retentionPending: false };
      if (command === 'get_lifecycle_status') return { launchAtLoginAvailable: true, launchAtLoginEnabled: false, hotkeysPaused: false };
      if (command === 'list_history' || command === 'list_recoverable_jobs') return [];
      if (command === 'check_for_update') return {
        available: true,
        version: '0.1.4',
        releaseNotes: '단축키 저장 안정화',
        publishedAt: '2026-09-02T00:00:00Z',
        sizeBytes: 1024,
        consentToken: 'unused-until-manual-confirmation',
      };
      if (command === 'mark_app_healthy') return true;
      throw new Error(`unexpected command: ${command}`);
    });
    render(<StrictMode><App /></StrictMode>);

    const button = await screen.findByRole('button', { name: '알림 1개' });
    await userEvent.click(button);

    expect(screen.getByRole('dialog', { name: '알림' })).toHaveTextContent('새 버전 0.1.4를 사용할 수 있습니다. 설정의 업데이트에서 확인하세요.');
    expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === 'check_for_update')).toHaveLength(1);
    expect(invoke).not.toHaveBeenCalledWith('prepare_update', expect.anything());
    expect(invoke).not.toHaveBeenCalledWith('install_update', expect.anything());
  });

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
    expect(screen.queryByRole('heading', { name: 'SmartCAT Translate' })).not.toBeInTheDocument();
    expect(await screen.findByLabelText('원문')).toBeVisible();
    expect(screen.getByLabelText('번역문')).toBeVisible();
    const navigation = screen.getByRole('tablist', { name: '주요 화면' });
    expect(within(navigation).getByRole('tab', { name: '텍스트' })).toHaveAttribute('aria-selected', 'true');
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
    expect(screen.queryByRole('complementary', { name: 'ChatGPT account' })).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Open menu' }));
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
    expect(screen.getByRole('button', { name: 'Open menu' })).toBeVisible();
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
    const navigation = await screen.findByRole('tablist', { name: '주요 화면' });
    const translateTab = within(navigation).getByRole('tab', { name: '텍스트' });
    expect(translateTab).toHaveAttribute('aria-controls', 'app-panel-translate');
    expect(container.querySelector('main')).toHaveAttribute('data-theme', 'dark');

    await userEvent.click(screen.getByRole('button', { name: '메뉴 열기' }));
    await userEvent.click(screen.getByRole('button', { name: '일반 설정' }));
    expect(screen.getByRole('tabpanel', { name: '설정' })).toBeVisible();
    expect(screen.getByRole('heading', { name: '설정' })).toBeVisible();
    expect(screen.getByLabelText('화면 언어')).toBeVisible();
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

    await user.click(screen.getByRole('button', { name: '메뉴 열기' }));
    const settingsButton = screen.getByRole('button', { name: '일반 설정' });
    await waitFor(() => expect(settingsButton).toBeDisabled());
    expect(screen.getByText('번역 또는 취소 처리 중에는 설정을 열 수 없습니다.')).toBeVisible();
    await user.click(settingsButton);
    expect(source).toBeInTheDocument();
    expect(screen.queryByRole('heading', { name: '설정' })).not.toBeInTheDocument();

    act(() => eventBridge.translation?.({ payload: {
      type: 'completed', jobId: 'job-1', result: { translatedText: '안녕하세요', detectedLanguage: 'en' },
    } }));
    await waitFor(() => expect(settingsButton).toBeEnabled());
    await user.click(settingsButton);
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
    await user.click(screen.getByRole('button', { name: 'Open menu' }));
    expect(screen.getByRole('button', { name: 'General settings' })).toBeDisabled();
  });

  it('moves completed image OCR into the text workspace without translating it again', async () => {
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'get_settings') return {
        schemaVersion: 1, locale: 'en', theme: 'system', defaultProfileId: 'default-profile',
        profiles: [{ id: 'default-profile', name: 'Default profile', field: 'general', profile: { sourceLanguage: null, targetLanguage: 'ko', quality: 'balanced', tone: 'natural', protectedTerms: [] } }],
        glossary: [], selectedModel: { type: 'automatic' }, launchAtLogin: false, closeBehavior: 'keepInTray', quickAccessPosition: 'popup', historyRetentionDays: 30,
      };
      if (command === 'get_account') return { account: { state: 'signedIn' }, loginPending: false };
      if (command === 'get_privacy_status') return { cleanupPending: false, retentionPending: false };
      if (command === 'get_lifecycle_status') return { launchAtLoginAvailable: true, launchAtLoginEnabled: false, hotkeysPaused: false };
      if (command === 'list_history' || command === 'list_recoverable_jobs') return [];
      if (command === 'choose_image') return {
        jobId: 'capture-job', status: 'sourceReady', imageWidth: 800, imageHeight: 600,
        translatedBlocks: [], warnings: [],
      };
      if (command === 'translate_image') return {
        jobId: 'capture-job', status: 'rendered', imageWidth: 800, imageHeight: 600,
        translatedBlocks: [
          { id: 'one', sourceIds: ['line-one'], sourceText: 'First source line', translatedText: 'First translated line', bounds: { x: 0, y: 0, width: 0.5, height: 0.1 }, confidence: 0.99, visible: true },
          { id: 'two', sourceIds: ['line-two'], sourceText: 'Second source line', translatedText: 'Second translated line', bounds: { x: 0, y: 0.2, width: 0.5, height: 0.1 }, confidence: 0.98, visible: false },
        ],
        warnings: [], sourcePreview: 'data:image/png;base64,source', translatedPreview: 'data:image/png;base64,translated',
      };
      throw new Error(`unexpected command: ${command}`);
    });
    const user = userEvent.setup();
    render(<App locale="en" />);

    const navigation = await screen.findByRole('tablist', { name: 'Main views' });
    await user.click(within(navigation).getByRole('tab', { name: 'Image & screen' }));
    await user.click(screen.getByRole('button', { name: 'Open image file' }));

    await waitFor(() => expect(within(navigation).getByRole('tab', { name: 'Text' })).toHaveAttribute('aria-selected', 'true'));
    expect(screen.getByLabelText('Source text')).toHaveValue('First source line\n\nSecond source line');
    expect(screen.getByLabelText('Translation')).toHaveValue('First translated line\n\nSecond translated line');
    expect(vi.mocked(invoke).mock.calls.some(([command]) => command === 'translate_text')).toBe(false);
    expect(vi.mocked(invoke).mock.calls.some(([command]) => command === 'save_history_record')).toBe(false);
  });
});
