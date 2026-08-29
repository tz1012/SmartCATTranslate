import { act, cleanup, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { invoke } from '@tauri-apps/api/core';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { SettingsView, type AppSettings } from './SettingsView';

const accountEvents = vi.hoisted(() => ({
  handler: null as null | ((reason: 'loginSucceeded' | 'loginFailed' | 'loginCancelled' | 'accountUpdated') => void),
  unlisten: vi.fn(),
}));

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((resolvePromise) => { resolve = resolvePromise; });
  return { promise, resolve };
}

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('../account/accountApi', () => ({
  onAccountStateChanged: vi.fn(async (handler: typeof accountEvents.handler) => {
    accountEvents.handler = handler;
    return accountEvents.unlisten;
  }),
}));

const defaultSettings: AppSettings = {
  schemaVersion: 1,
  locale: 'ko',
  theme: 'system',
  defaultProfileId: 'default-profile',
  profiles: [{
    id: 'default-profile',
    name: '기본 프로필',
    field: 'general',
    profile: {
      sourceLanguage: null,
      targetLanguage: 'ko',
      quality: 'balanced',
      tone: 'natural',
      protectedTerms: [],
    },
  }],
  glossary: [],
  selectedModel: { type: 'automatic' },
  launchAtLogin: false,
  closeBehavior: 'keepInTray',
  quickAccessPosition: 'popup',
  historyRetentionDays: 30,
};

function mockCommands(settings: AppSettings = defaultSettings, models: unknown[] = []) {
  vi.mocked(invoke).mockImplementation(async (command, args) => {
    if (command === 'get_settings') return structuredClone(settings);
    if (command === 'list_available_models') return models;
    if (command === 'save_settings') return structuredClone((args as { settings: AppSettings }).settings);
    throw new Error(`unexpected command: ${command}`);
  });
}

afterEach(() => {
  cleanup();
  accountEvents.handler = null;
  vi.clearAllMocks();
});

describe('SettingsView', () => {
  it('shows approved Korean defaults and all settings groups', async () => {
    mockCommands();
    render(<SettingsView />);

    expect(await screen.findByRole('heading', { name: '설정' })).toBeVisible();
    const profiles = screen.getByRole('group', { name: '번역 프로필' });
    expect(within(profiles).getByLabelText('원문 언어')).toHaveValue('auto');
    expect(within(profiles).getByLabelText('대상 언어')).toHaveValue('ko');
    expect(within(profiles).getByLabelText('품질')).toHaveValue('balanced');
    expect(within(profiles).getByLabelText('문체')).toHaveValue('natural');
    expect(within(profiles).getByLabelText('분야')).toHaveValue('general');
    expect(screen.getByRole('group', { name: '용어집' })).toBeVisible();
    expect(screen.getByRole('group', { name: '모델' })).toBeVisible();
    expect(screen.getByLabelText('로그인할 때 실행')).not.toBeChecked();
    expect(screen.getByLabelText('닫기 동작')).toHaveValue('keepInTray');
    expect(screen.getByLabelText('빠른 번역 위치')).toHaveValue('popup');
  });

  it('creates, renames and deletes a profile before saving', async () => {
    mockCommands();
    const user = userEvent.setup();
    render(<SettingsView />);
    await screen.findByRole('heading', { name: '설정' });

    await user.click(screen.getByRole('button', { name: '프로필 추가' }));
    const profiles = screen.getByRole('group', { name: '번역 프로필' });
    const name = within(profiles).getByLabelText('프로필 이름');
    await user.clear(name);
    await user.type(name, '제품 문서');
    await user.click(within(profiles).getByRole('button', { name: '프로필 삭제' }));
    await user.click(screen.getByRole('button', { name: '설정 저장' }));

    expect(invoke).toHaveBeenCalledWith('save_settings', {
      settings: expect.objectContaining({ profiles: [expect.objectContaining({ id: 'default-profile' })] }),
    });
  });

  it('rejects a duplicate glossary source term and supports protected terms', async () => {
    mockCommands();
    const user = userEvent.setup();
    render(<SettingsView />);
    await screen.findByRole('heading', { name: '설정' });

    await user.type(screen.getByLabelText('원문 용어'), 'SmartCAT');
    await user.click(screen.getByLabelText('번역하지 않고 보호'));
    await user.click(screen.getByRole('button', { name: '용어 추가' }));
    await user.type(screen.getByLabelText('원문 용어'), ' smartcat ');
    await user.click(screen.getByRole('button', { name: '용어 추가' }));

    expect(screen.getByRole('alert')).toHaveTextContent('같은 원문 용어가 이미 있습니다');
    expect(screen.getAllByRole('row')).toHaveLength(2);
  });

  it('warns and uses automatic when the saved model is unavailable without erasing it', async () => {
    mockCommands({ ...defaultSettings, selectedModel: { type: 'specific', id: 'retired-model' } }, [
      { id: 'available', displayName: 'Available', supportedReasoningEfforts: ['low'], isDefault: true },
    ]);
    render(<SettingsView />);

    expect(await screen.findByText('사용할 수 없어 자동 선택으로 전환됨')).toBeVisible();
    expect(screen.getByLabelText('모델 선택')).toHaveValue('automatic');
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('list_available_models'));
  });

  it('preserves a saved model when the authoritative model list cannot be loaded', async () => {
    const settings = { ...defaultSettings, selectedModel: { type: 'specific', id: 'saved-model' } as const };
    vi.mocked(invoke).mockImplementation(async (command, args) => {
      if (command === 'get_settings') return structuredClone(settings);
      if (command === 'list_available_models') throw new Error('offline');
      if (command === 'save_settings') return structuredClone((args as { settings: AppSettings }).settings);
      throw new Error(`unexpected command: ${command}`);
    });
    render(<SettingsView />);

    expect(await screen.findByRole('status', { name: '모델 목록 상태' })).toHaveTextContent('모델 목록을 불러올 수 없습니다');
    await userEvent.click(screen.getByRole('button', { name: '설정 저장' }));

    expect(invoke).toHaveBeenCalledWith('save_settings', {
      settings: expect.objectContaining({ selectedModel: { type: 'specific', id: 'saved-model' } }),
    });
  });

  it('refreshes a signed-out model catalog after the account login event', async () => {
    let modelReads = 0;
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'get_settings') return structuredClone(defaultSettings);
      if (command === 'list_available_models') {
        modelReads += 1;
        if (modelReads === 1) throw 'model_catalog_signed_out';
        return [{ id: 'connected-model', displayName: 'Connected Model', supportedReasoningEfforts: [], isDefault: true }];
      }
      throw new Error(`unexpected command: ${command}`);
    });
    render(<SettingsView />);
    expect(await screen.findByRole('status', { name: '모델 목록 상태' })).toHaveTextContent('ChatGPT 계정 연결 후');

    await act(async () => accountEvents.handler?.('loginSucceeded'));

    expect(await screen.findByRole('option', { name: 'Connected Model' })).toBeVisible();
    expect(modelReads).toBe(2);
  });

  it('offers localized retry and uses it to refresh a failed catalog', async () => {
    let modelReads = 0;
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'get_settings') return structuredClone(defaultSettings);
      if (command === 'list_available_models') {
        modelReads += 1;
        if (modelReads === 1) throw new Error('offline');
        return [{ id: 'retry-model', displayName: 'Retry Model', supportedReasoningEfforts: [], isDefault: true }];
      }
      throw new Error(`unexpected command: ${command}`);
    });
    const user = userEvent.setup();
    render(<SettingsView />);

    await user.click(await screen.findByRole('button', { name: '다시 시도' }));

    expect(await screen.findByRole('option', { name: 'Retry Model' })).toBeVisible();
    expect(modelReads).toBe(2);

    cleanup();
    vi.clearAllMocks();
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'get_settings') return structuredClone({ ...defaultSettings, locale: 'en' });
      if (command === 'list_available_models') throw new Error('offline');
      throw new Error(`unexpected command: ${command}`);
    });
    render(<SettingsView />);
    expect(await screen.findByRole('button', { name: 'Retry' })).toBeVisible();
  });

  it('ignores an older retry result after a newer account refresh completes', async () => {
    const older = deferred<unknown[]>();
    const newer = deferred<unknown[]>();
    let modelReads = 0;
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'get_settings') return structuredClone(defaultSettings);
      if (command === 'list_available_models') {
        modelReads += 1;
        if (modelReads === 1) throw new Error('offline');
        return modelReads === 2 ? older.promise : newer.promise;
      }
      throw new Error(`unexpected command: ${command}`);
    });
    const user = userEvent.setup();
    render(<SettingsView />);
    await user.click(await screen.findByRole('button', { name: '다시 시도' }));
    await waitFor(() => expect(modelReads).toBe(2));

    await act(async () => accountEvents.handler?.('accountUpdated'));
    await waitFor(() => expect(modelReads).toBe(3));
    newer.resolve([{ id: 'new-model', displayName: 'New Model', supportedReasoningEfforts: [], isDefault: true }]);
    expect(await screen.findByRole('option', { name: 'New Model' })).toBeVisible();
    older.resolve([{ id: 'old-model', displayName: 'Old Model', supportedReasoningEfforts: [], isDefault: true }]);
    await act(async () => older.promise);

    expect(screen.getByRole('option', { name: 'New Model' })).toBeVisible();
    expect(screen.queryByRole('option', { name: 'Old Model' })).not.toBeInTheDocument();
  });

  it('unsubscribes and ignores account events after unmount', async () => {
    mockCommands();
    const view = render(<SettingsView />);
    await screen.findByRole('heading', { name: '설정' });
    await waitFor(() => expect(accountEvents.handler).not.toBeNull());
    const modelReads = vi.mocked(invoke).mock.calls.filter(([command]) => command === 'list_available_models').length;

    view.unmount();
    await act(async () => accountEvents.handler?.('accountUpdated'));

    expect(accountEvents.unlisten).toHaveBeenCalledOnce();
    expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === 'list_available_models')).toHaveLength(modelReads);
  });

  it('offers rewrite or target-language change for a matching detected language', async () => {
    mockCommands();
    const onRewrite = vi.fn();
    render(<SettingsView detectedSourceLanguage="ko" onRewrite={onRewrite} />);

    expect(await screen.findByText('문장을 개선할까요?')).toBeVisible();
    await userEvent.click(screen.getByRole('button', { name: '문장 개선' }));
    expect(onRewrite).toHaveBeenCalledOnce();
    expect(screen.getByRole('button', { name: '대상 언어 변경' })).toBeVisible();
  });

  it('offers rewrite when an explicitly selected source equals the target', async () => {
    mockCommands();
    render(<SettingsView />);
    const profiles = await screen.findByRole('group', { name: '번역 프로필' });

    await userEvent.selectOptions(within(profiles).getByLabelText('원문 언어'), 'ko');

    expect(screen.getByText('문장을 개선할까요?')).toBeVisible();
  });

  it('ignores an older save response after a newer edit and save', async () => {
    const first = deferred<AppSettings>();
    const second = deferred<AppSettings>();
    let saveIndex = 0;
    mockCommands();
    vi.mocked(invoke).mockImplementation(async (command, args) => {
      if (command === 'get_settings') return structuredClone(defaultSettings);
      if (command === 'list_available_models') return [];
      if (command === 'save_settings') {
        saveIndex += 1;
        return saveIndex === 1 ? first.promise : second.promise;
      }
      throw new Error(`unexpected command: ${command} ${String(args)}`);
    });
    const user = userEvent.setup();
    render(<SettingsView />);
    await screen.findByRole('heading', { name: '설정' });

    await user.click(screen.getByRole('button', { name: '설정 저장' }));
    await user.selectOptions(screen.getByLabelText('테마'), 'dark');
    await user.click(screen.getByRole('button', { name: '설정 저장' }));
    second.resolve({ ...defaultSettings, theme: 'dark' });
    await waitFor(() => expect(screen.getByLabelText('테마')).toHaveValue('dark'));
    await act(async () => {
      first.resolve({ ...defaultSettings, theme: 'system' });
      await first.promise;
    });
    expect(screen.getByLabelText('테마')).toHaveValue('dark');
  });

  it('supports saving a language pair beyond Korean and English', async () => {
    mockCommands();
    const user = userEvent.setup();
    render(<SettingsView />);
    const profiles = await screen.findByRole('group', { name: '번역 프로필' });

    await user.selectOptions(within(profiles).getByLabelText('대상 언어'), 'ja');
    await user.click(screen.getByRole('button', { name: '설정 저장' }));

    expect(invoke).toHaveBeenCalledWith('save_settings', {
      settings: expect.objectContaining({
        profiles: [expect.objectContaining({
          profile: expect.objectContaining({ targetLanguage: 'ja' }),
        })],
      }),
    });
  });

  it('renders English UI without Korean labels', async () => {
    mockCommands({ ...defaultSettings, locale: 'en' });
    const { container } = render(<SettingsView />);
    expect(await screen.findByRole('heading', { name: 'Settings' })).toBeVisible();
    expect(within(screen.getByRole('group', { name: 'Translation profiles' })).getByLabelText('Source language')).toBeVisible();
    expect(container.textContent).not.toMatch(/[\u3131-\u318E\uAC00-\uD7A3]/u);
  });
});
