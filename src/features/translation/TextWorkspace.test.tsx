import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { AppSettings } from '../settings/SettingsView';
import { TextWorkspace } from './TextWorkspace';

type EventCallback = (event: { payload: unknown }) => void;

const bridge = vi.hoisted(() => ({
  translationHandler: null as EventCallback | null,
  accountHandler: null as EventCallback | null,
  unlistenTranslation: vi.fn(),
  unlistenAccount: vi.fn(),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (name: string, handler: EventCallback) => {
    if (name === 'translation-event') {
      bridge.translationHandler = handler;
      return bridge.unlistenTranslation;
    }
    if (name === 'account-state-changed') {
      bridge.accountHandler = handler;
      return bridge.unlistenAccount;
    }
    throw new Error(`unexpected event: ${name}`);
  }),
}));

const settings: AppSettings = {
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

function signedInCommands() {
  vi.mocked(invoke).mockImplementation(async (command) => {
    if (command === 'get_settings') return structuredClone(settings);
    if (command === 'get_account') {
      return { account: { state: 'signedIn', emailHint: null, plan: null }, loginPending: false };
    }
    if (command === 'translate_text') return 'job-1';
    if (command === 'cancel_translation') return true;
    throw new Error(`unexpected command: ${command}`);
  });
}

function emitTranslation(payload: unknown) {
  act(() => bridge.translationHandler?.({ payload }));
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

beforeEach(() => {
  signedInCommands();
});

afterEach(() => {
  cleanup();
  bridge.translationHandler = null;
  bridge.accountHandler = null;
  vi.clearAllMocks();
});

describe('TextWorkspace', () => {
  it('translates with the approved saved default profile', async () => {
    const user = userEvent.setup();
    render(<TextWorkspace />);

    await user.type(await screen.findByLabelText('원문'), 'Hello');
    await user.click(screen.getByRole('button', { name: '번역' }));

    expect(invoke).toHaveBeenCalledWith('translate_text', {
      request: {
        text: 'Hello',
        profile: {
          sourceLanguage: null,
          targetLanguage: 'ko',
          quality: 'balanced',
          tone: 'natural',
          protectedTerms: [],
        },
        mode: 'translate',
        secret: false,
      },
    });
  });

  it('streams only the active job and replaces deltas with the completed result', async () => {
    render(<TextWorkspace />);
    await userEvent.type(await screen.findByLabelText('원문'), 'Hello');
    await userEvent.click(screen.getByRole('button', { name: '번역' }));
    await waitFor(() => expect(listen).toHaveBeenCalledWith('translation-event', expect.any(Function)));

    emitTranslation({ type: 'delta', jobId: 'other-job', text: 'wrong' });
    emitTranslation({ type: 'delta', jobId: 'job-1', text: '안녕' });
    emitTranslation({ type: 'delta', jobId: 'job-1', text: '하세요' });
    expect(screen.getByLabelText('번역문')).toHaveValue('안녕하세요');

    emitTranslation({
      type: 'completed',
      jobId: 'job-1',
      result: { translatedText: '안녕하세요.', detectedLanguage: 'en' },
    });
    expect(screen.getByLabelText('번역문')).toHaveValue('안녕하세요.');
    emitTranslation({ type: 'delta', jobId: 'job-1', text: 'stale' });
    expect(screen.getByLabelText('번역문')).toHaveValue('안녕하세요.');
  });

  it('does not lose an event emitted before translate_text returns the job id', async () => {
    const response = deferred<string>();
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'get_settings') return structuredClone(settings);
      if (command === 'get_account') return { account: { state: 'signedIn', emailHint: null, plan: null }, loginPending: false };
      if (command === 'translate_text') return response.promise;
      throw new Error(`unexpected command: ${command}`);
    });
    render(<TextWorkspace />);
    await userEvent.type(await screen.findByLabelText('원문'), 'Hello');
    await userEvent.click(screen.getByRole('button', { name: '번역' }));
    emitTranslation({ type: 'delta', jobId: 'early-job', text: '먼저 온 결과' });
    response.resolve('early-job');

    expect(await screen.findByDisplayValue('먼저 온 결과')).toBeVisible();
  });

  it('cancels a running job on the second action', async () => {
    render(<TextWorkspace />);
    await userEvent.type(await screen.findByLabelText('원문'), 'Hello');
    await userEvent.click(screen.getByRole('button', { name: '번역' }));
    await userEvent.click(await screen.findByRole('button', { name: '취소' }));
    expect(invoke).toHaveBeenCalledWith('cancel_translation', { jobId: 'job-1' });
  });

  it('unlistens and cancels a running job when unmounted', async () => {
    const view = render(<TextWorkspace />);
    await userEvent.type(await screen.findByLabelText('원문'), 'Hello');
    await userEvent.click(screen.getByRole('button', { name: '번역' }));

    view.unmount();
    expect(bridge.unlistenTranslation).toHaveBeenCalledOnce();
    expect(bridge.unlistenAccount).toHaveBeenCalledOnce();
    expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === 'cancel_translation')).toHaveLength(1);
  });

  it('prevents duplicate starts while the first start is pending', async () => {
    const response = deferred<string>();
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'get_settings') return structuredClone(settings);
      if (command === 'get_account') return { account: { state: 'signedIn', emailHint: null, plan: null }, loginPending: false };
      if (command === 'translate_text') return response.promise;
      throw new Error(`unexpected command: ${command}`);
    });
    render(<TextWorkspace />);
    await userEvent.type(await screen.findByLabelText('원문'), 'Hello');
    const translate = screen.getByRole('button', { name: '번역' });
    await act(async () => {
      translate.click();
      translate.click();
    });
    expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === 'translate_text')).toHaveLength(1);
    response.resolve('job-1');
  });

  it('cancels a start that is still waiting for its job id', async () => {
    const response = deferred<string>();
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'get_settings') return structuredClone(settings);
      if (command === 'get_account') return { account: { state: 'signedIn', emailHint: null, plan: null }, loginPending: false };
      if (command === 'translate_text') return response.promise;
      if (command === 'cancel_translation') return true;
      throw new Error(`unexpected command: ${command}`);
    });
    render(<TextWorkspace />);
    await userEvent.type(await screen.findByLabelText('원문'), 'Hello');
    await userEvent.click(screen.getByRole('button', { name: '번역' }));
    await userEvent.click(screen.getByRole('button', { name: '취소' }));
    response.resolve('late-job');

    await waitFor(() => expect(invoke).toHaveBeenCalledWith('cancel_translation', { jobId: 'late-job' }));
  });

  it('copies, saves and clears a completed result without storing it in app history', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } });
    Object.defineProperty(URL, 'createObjectURL', { configurable: true, value: vi.fn() });
    Object.defineProperty(URL, 'revokeObjectURL', { configurable: true, value: vi.fn() });
    const createObjectURL = vi.spyOn(URL, 'createObjectURL').mockReturnValue('blob:translation');
    const revokeObjectURL = vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => undefined);
    vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => undefined);
    render(<TextWorkspace />);
    await userEvent.type(await screen.findByLabelText('원문'), 'Hello');
    await userEvent.click(screen.getByRole('button', { name: '번역' }));
    emitTranslation({ type: 'completed', jobId: 'job-1', result: { translatedText: '안녕하세요', detectedLanguage: 'en' } });

    await userEvent.click(screen.getByRole('button', { name: '번역문 복사' }));
    expect(writeText).toHaveBeenCalledWith('안녕하세요');
    await userEvent.click(screen.getByRole('button', { name: '번역문 저장' }));
    expect(createObjectURL).toHaveBeenCalledOnce();
    expect(revokeObjectURL).toHaveBeenCalledWith('blob:translation');
    await userEvent.click(screen.getByRole('button', { name: '모두 지우기' }));
    expect(screen.getByLabelText('원문')).toHaveValue('');
    expect(screen.getByLabelText('번역문')).toHaveValue('');
  });

  it('validates empty and oversized sources before invoking the backend', async () => {
    render(<TextWorkspace />);
    await screen.findByLabelText('원문');
    await userEvent.click(screen.getByRole('button', { name: '번역' }));
    expect(screen.getByRole('alert')).toHaveTextContent('번역할 원문을 입력해 주세요.');

    fireEvent.change(screen.getByLabelText('원문'), { target: { value: 'a'.repeat(200_001) } });
    await userEvent.click(screen.getByRole('button', { name: '번역' }));
    expect(screen.getByRole('alert')).toHaveTextContent('원문은 200,000자');
    expect(invoke).not.toHaveBeenCalledWith('translate_text', expect.anything());
  });

  it('disables translation while signed out and refreshes after account change', async () => {
    let signedIn = false;
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'get_settings') return structuredClone(settings);
      if (command === 'get_account') return { account: { state: signedIn ? 'signedIn' : 'signedOut' }, loginPending: false };
      if (command === 'translate_text') return 'job-1';
      throw new Error(`unexpected command: ${command}`);
    });
    render(<TextWorkspace />);
    expect(await screen.findByText('번역하려면 ChatGPT 계정을 연결해 주세요.')).toBeVisible();
    expect(screen.getByRole('button', { name: '번역' })).toBeDisabled();

    signedIn = true;
    act(() => bridge.accountHandler?.({ payload: { reason: 'loginSucceeded' } }));
    await waitFor(() => expect(screen.getByRole('button', { name: '번역' })).toBeEnabled());
  });

  it('announces a localized failure and retries the unchanged request', async () => {
    render(<TextWorkspace />);
    await userEvent.type(await screen.findByLabelText('원문'), 'Hello');
    await userEvent.click(screen.getByRole('button', { name: '번역' }));
    emitTranslation({ type: 'failed', jobId: 'job-1', code: 'translation_timed_out', message: 'private backend message' });

    expect(screen.getByRole('alert')).toHaveTextContent('번역 시간이 초과되었습니다.');
    expect(screen.queryByText('private backend message')).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: '다시 시도' }));
    expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === 'translate_text')).toHaveLength(2);
  });

  it('swaps an explicit language pair and offers rewrite for matching languages', async () => {
    const explicit = structuredClone(settings);
    explicit.profiles[0].profile.sourceLanguage = 'en';
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'get_settings') return explicit;
      if (command === 'get_account') return { account: { state: 'signedIn', emailHint: null, plan: null }, loginPending: false };
      if (command === 'translate_text') return 'job-1';
      throw new Error(`unexpected command: ${command}`);
    });
    render(<TextWorkspace />);
    expect(await screen.findByLabelText('원문 언어')).toHaveValue('en');
    expect(screen.getByLabelText('대상 언어')).toHaveValue('ko');
    await userEvent.click(screen.getByRole('button', { name: '언어 바꾸기' }));
    expect(screen.getByLabelText('원문 언어')).toHaveValue('ko');
    expect(screen.getByLabelText('대상 언어')).toHaveValue('en');

    await userEvent.selectOptions(screen.getByLabelText('대상 언어'), 'ko');
    expect(screen.getByText('같은 언어입니다. 문장을 개선할까요?')).toBeVisible();
    await userEvent.type(screen.getByLabelText('원문'), '문장');
    await userEvent.click(screen.getByRole('button', { name: '문장 개선' }));
    expect(invoke).toHaveBeenCalledWith('translate_text', {
      request: expect.objectContaining({ mode: 'rewrite', text: '문장' }),
    });
  });

  it('uses matching protected glossary terms from the saved settings', async () => {
    const withGlossary = structuredClone(settings);
    withGlossary.profiles[0].profile.sourceLanguage = 'en';
    withGlossary.glossary = [{
      id: 'term-1', sourceLanguage: 'en', targetLanguage: 'ko', sourceTerm: 'SmartCAT', targetTerm: '', protectOnly: true,
    }];
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'get_settings') return withGlossary;
      if (command === 'get_account') return { account: { state: 'signedIn', emailHint: null, plan: null }, loginPending: false };
      if (command === 'translate_text') return 'job-1';
      throw new Error(`unexpected command: ${command}`);
    });
    render(<TextWorkspace />);
    await userEvent.type(await screen.findByLabelText('원문'), 'SmartCAT');
    await userEvent.click(screen.getByRole('button', { name: '번역' }));

    expect(invoke).toHaveBeenCalledWith('translate_text', {
      request: expect.objectContaining({
        profile: expect.objectContaining({ protectedTerms: ['SmartCAT'] }),
      }),
    });
  });

  it('renders the compact workspace and status labels entirely in English', async () => {
    const englishSettings = structuredClone(settings);
    englishSettings.locale = 'en';
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'get_settings') return englishSettings;
      if (command === 'get_account') return { account: { state: 'signedIn', emailHint: null, plan: null }, loginPending: false };
      throw new Error(`unexpected command: ${command}`);
    });
    const { container } = render(<TextWorkspace locale="en" />);
    expect(await screen.findByRole('region', { name: 'Text translation' })).toBeVisible();
    expect(screen.getByRole('tab', { name: 'Text' })).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByText('Shortcut: Not set')).toBeVisible();
    expect(container.textContent).not.toMatch(/[\u3131-\u318E\uAC00-\uD7A3]/u);
  });
});
