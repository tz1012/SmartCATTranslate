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
    if (command === 'save_translation_text') return { status: 'saved' };
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
  vi.mocked(listen).mockImplementation((async (name: string, handler: EventCallback) => {
    if (name === 'translation-event') {
      bridge.translationHandler = handler;
      return bridge.unlistenTranslation;
    }
    if (name === 'account-state-changed') {
      bridge.accountHandler = handler;
      return bridge.unlistenAccount;
    }
    throw new Error(`unexpected event: ${name}`);
  }) as unknown as typeof listen);
  signedInCommands();
});

afterEach(() => {
  cleanup();
  bridge.translationHandler = null;
  bridge.accountHandler = null;
  vi.clearAllMocks();
});

describe('TextWorkspace', () => {
  it('uses the app top bar as the only workspace navigation', async () => {
    const { container } = render(<TextWorkspace />);

    await screen.findByLabelText('원문');
    expect(screen.queryByRole('tablist', { name: '텍스트 번역' })).not.toBeInTheDocument();
    expect(screen.queryByText('단축키: 설정되지 않음')).not.toBeInTheDocument();
    expect(container.querySelector('.workspace-footer')).toContainElement(screen.getByRole('button', { name: '번역' }));
  });

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
        field: 'general',
        glossary: [],
        mode: 'translate',
        secret: false,
      },
    });
  });

  it('starts translation immediately when text is pasted into the source pane', async () => {
    render(<TextWorkspace />);
    const source = await screen.findByLabelText('원문');

    fireEvent.paste(source, { clipboardData: { getData: () => 'Hello from paste' } });

    await waitFor(() => expect(invoke).toHaveBeenCalledWith('translate_text', {
      request: expect.objectContaining({ text: 'Hello from paste', mode: 'translate' }),
    }));
    expect(source).toHaveValue('Hello from paste');
  });

  it('shows a safe service error when translation cannot start', async () => {
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'get_settings') return structuredClone(settings);
      if (command === 'get_account') return { account: { state: 'signedIn' }, loginPending: false };
      if (command === 'translate_text') throw 'translation_service_unavailable';
      throw new Error(`unexpected command: ${command}`);
    });
    render(<TextWorkspace />);
    await userEvent.type(await screen.findByLabelText('원문'), 'Hello');

    await userEvent.click(screen.getByRole('button', { name: '번역' }));

    expect(await screen.findByRole('alert')).toHaveTextContent('번역 서비스를 시작할 수 없습니다.');
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

  it('does not start before the event listener is ready and localizes listener failure', async () => {
    const listener = deferred<() => void>();
    vi.mocked(listen).mockImplementation(async (name, handler) => {
      if (name === 'translation-event') {
        bridge.translationHandler = handler as EventCallback;
        return listener.promise;
      }
      bridge.accountHandler = handler as EventCallback;
      return bridge.unlistenAccount;
    });
    render(<TextWorkspace />);
    await userEvent.type(await screen.findByLabelText('원문'), 'Hello');
    expect(screen.getByRole('button', { name: '번역' })).toBeDisabled();
    listener.reject(new Error('private listener detail'));

    expect(await screen.findByRole('alert')).toHaveTextContent('번역 서비스를 시작할 수 없습니다.');
    expect(screen.queryByText('private listener detail')).not.toBeInTheDocument();
    expect(invoke).not.toHaveBeenCalledWith('translate_text', expect.anything());
  });

  it('cancels a running job on the second action', async () => {
    render(<TextWorkspace />);
    await userEvent.type(await screen.findByLabelText('원문'), 'Hello');
    await userEvent.click(screen.getByRole('button', { name: '번역' }));
    await userEvent.click(await screen.findByRole('button', { name: '취소' }));
    expect(invoke).toHaveBeenCalledWith('cancel_translation', { jobId: 'job-1' });
  });

  it('keeps cancellation enabled when the account becomes signed out', async () => {
    let signedIn = true;
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'get_settings') return structuredClone(settings);
      if (command === 'get_account') return { account: { state: signedIn ? 'signedIn' : 'signedOut' }, loginPending: false };
      if (command === 'translate_text') return 'job-1';
      if (command === 'cancel_translation') return true;
      throw new Error(`unexpected command: ${command}`);
    });
    render(<TextWorkspace />);
    await userEvent.type(await screen.findByLabelText('원문'), 'Hello');
    await userEvent.click(screen.getByRole('button', { name: '번역' }));
    signedIn = false;
    act(() => bridge.accountHandler?.({ payload: { reason: 'accountUpdated' } }));

    const cancelButton = await screen.findByRole('button', { name: '취소' });
    expect(cancelButton).toBeEnabled();
    await userEvent.click(cancelButton);
    expect(invoke).toHaveBeenCalledWith('cancel_translation', { jobId: 'job-1' });
  });

  it.each(['false', 'error'] as const)('reports a %s cancellation failure and retries cleanup', async (failure) => {
    let cancelAttempts = 0;
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'get_settings') return structuredClone(settings);
      if (command === 'get_account') return { account: { state: 'signedIn' }, loginPending: false };
      if (command === 'translate_text') return 'job-1';
      if (command === 'cancel_translation') {
        cancelAttempts += 1;
        if (cancelAttempts === 1) {
          if (failure === 'error') throw new Error('private cancel detail');
          return false;
        }
        return true;
      }
      throw new Error(`unexpected command: ${command}`);
    });
    render(<TextWorkspace />);
    await userEvent.type(await screen.findByLabelText('원문'), 'Hello');
    await userEvent.click(screen.getByRole('button', { name: '번역' }));
    await userEvent.click(screen.getByRole('button', { name: '취소' }));

    expect(await screen.findByRole('alert')).toHaveTextContent('번역 취소를 완료하지 못했습니다.');
    expect(screen.queryByText('private cancel detail')).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: '다시 시도' }));
    expect(cancelAttempts).toBe(2);
    expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === 'translate_text')).toHaveLength(1);
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

  it('copies, saves natively and clears a completed result without storing it in app history', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } });
    render(<TextWorkspace />);
    await userEvent.type(await screen.findByLabelText('원문'), 'Hello');
    await userEvent.click(screen.getByRole('button', { name: '번역' }));
    emitTranslation({ type: 'completed', jobId: 'job-1', result: { translatedText: '안녕하세요', detectedLanguage: 'en' } });

    await userEvent.click(screen.getByRole('button', { name: '번역문 복사' }));
    expect(writeText).toHaveBeenCalledWith('안녕하세요');
    await userEvent.click(screen.getByRole('button', { name: '번역문 저장' }));
    expect(invoke).toHaveBeenCalledWith('save_translation_text', { text: '안녕하세요', targetLanguage: 'ko', locale: 'ko' });
    expect(screen.getByRole('status')).toHaveTextContent('번역문 파일을 저장했습니다.');
    await userEvent.click(screen.getByRole('button', { name: '모두 지우기' }));
    expect(screen.getByLabelText('원문')).toHaveValue('');
    expect(screen.getByLabelText('번역문')).toHaveValue('');
  });

  it('announces fixed copy and native save failures while treating save cancellation as neutral', async () => {
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText: vi.fn().mockRejectedValue(new Error('private copy detail')) } });
    let saveResult: unknown = { status: 'cancelled' };
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'get_settings') return structuredClone(settings);
      if (command === 'get_account') return { account: { state: 'signedIn' }, loginPending: false };
      if (command === 'translate_text') return 'job-1';
      if (command === 'save_translation_text') {
        if (saveResult instanceof Error) throw saveResult;
        return saveResult;
      }
      throw new Error(`unexpected command: ${command}`);
    });
    render(<TextWorkspace />);
    await userEvent.type(await screen.findByLabelText('원문'), 'Hello');
    await userEvent.click(screen.getByRole('button', { name: '번역' }));
    emitTranslation({ type: 'completed', jobId: 'job-1', result: { translatedText: '안녕하세요', detectedLanguage: 'en' } });

    await userEvent.click(screen.getByRole('button', { name: '번역문 복사' }));
    expect(screen.getByRole('alert')).toHaveTextContent('번역문을 복사하지 못했습니다.');
    await userEvent.click(screen.getByRole('button', { name: '번역문 저장' }));
    expect(screen.getByRole('status')).not.toHaveTextContent('저장했습니다');
    saveResult = new Error('private save detail');
    await userEvent.click(screen.getByRole('button', { name: '번역문 저장' }));
    expect(screen.getByRole('alert')).toHaveTextContent('번역문 파일을 저장하지 못했습니다.');
    expect(screen.queryByText(/private (copy|save) detail/)).not.toBeInTheDocument();
  });

  it('keeps source focus with readOnly and cancels on Escape after the keyboard start shortcut', async () => {
    const user = userEvent.setup();
    const activity: boolean[] = [];
    render(<TextWorkspace onActivityChange={(active) => activity.push(active)} />);
    const source = await screen.findByLabelText('원문');
    await user.type(source, 'Hello');
    source.focus();
    await user.keyboard('{Control>}{Enter}{/Control}');
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('translate_text', expect.anything()));
    expect(source).toHaveAttribute('readonly');
    expect(source).toHaveFocus();
    await user.keyboard('{Escape}');
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('cancel_translation', { jobId: 'job-1' }));
    expect(activity.at(-1)).toBe(true);
    emitTranslation({ type: 'failed', jobId: 'job-1', code: 'translation_cancelled', message: 'private' });
    await waitFor(() => expect(activity.at(-1)).toBe(false));
  });

  it('clears a completed result when the source changes', async () => {
    render(<TextWorkspace />);
    const source = await screen.findByLabelText('원문');
    await userEvent.type(source, 'Hello');
    await userEvent.click(screen.getByRole('button', { name: '번역' }));
    emitTranslation({ type: 'completed', jobId: 'job-1', result: { translatedText: '안녕하세요', detectedLanguage: 'en' } });
    await userEvent.type(source, '!');
    expect(screen.getByLabelText('번역문')).toHaveValue('');
  });

  it('reports authoritative activity and clears it on unmount without accepting stale events', async () => {
    const activity: boolean[] = [];
    const { unmount } = render(<TextWorkspace onActivityChange={(active) => activity.push(active)} />);
    await userEvent.type(await screen.findByLabelText('원문'), 'Hello');
    await userEvent.click(screen.getByRole('button', { name: '번역' }));
    await waitFor(() => expect(activity.at(-1)).toBe(true));

    emitTranslation({ type: 'failed', jobId: 'stale-job', code: 'translation_failed', message: 'stale' });
    expect(activity.at(-1)).toBe(true);
    emitTranslation({ type: 'failed', jobId: 'job-1', code: 'translation_failed', message: 'private' });
    await waitFor(() => expect(activity.at(-1)).toBe(false));
    await userEvent.click(screen.getByRole('button', { name: '다시 시도' }));
    await waitFor(() => expect(activity.at(-1)).toBe(true));

    unmount();
    expect(activity.at(-1)).toBe(false);
    emitTranslation({ type: 'completed', jobId: 'job-1', result: { translatedText: 'stale', detectedLanguage: 'en' } });
    expect(activity.at(-1)).toBe(false);
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

  it('passes the saved field and matching source-to-target glossary mappings', async () => {
    const withMapping = structuredClone(settings);
    withMapping.profiles[0].field = 'technical';
    withMapping.profiles[0].profile.sourceLanguage = 'en';
    withMapping.glossary = [
      { id: 'map-1', sourceLanguage: 'en', targetLanguage: 'ko', sourceTerm: 'cloud', targetTerm: '클라우드', protectOnly: false },
      { id: 'protect-1', sourceLanguage: 'en', targetLanguage: 'ko', sourceTerm: 'SmartCAT', targetTerm: '', protectOnly: true },
    ];
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'get_settings') return withMapping;
      if (command === 'get_account') return { account: { state: 'signedIn' }, loginPending: false };
      if (command === 'translate_text') return 'job-1';
      throw new Error(`unexpected command: ${command}`);
    });
    render(<TextWorkspace />);
    await userEvent.type(await screen.findByLabelText('원문'), 'cloud by SmartCAT');
    await userEvent.click(screen.getByRole('button', { name: '번역' }));

    expect(invoke).toHaveBeenCalledWith('translate_text', { request: expect.objectContaining({
      field: 'technical',
      glossary: [{ sourceTerm: 'cloud', targetTerm: '클라우드' }],
      profile: expect.objectContaining({ protectedTerms: ['SmartCAT'] }),
    }) });
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
    expect(screen.queryByRole('tablist', { name: 'Text translation' })).not.toBeInTheDocument();
    expect(screen.queryByText('Shortcut: Not set')).not.toBeInTheDocument();
    expect(screen.getByRole('status')).toHaveTextContent('Ready to translate.');
    expect(container.textContent).not.toMatch(/[\u3131-\u318E\uAC00-\uD7A3]/u);
  });
});
