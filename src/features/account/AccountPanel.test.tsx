import { act, cleanup, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { AccountPanel, formatResetTime } from './AccountPanel';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));

const invokeMock = vi.mocked(invoke);
const listenMock = vi.mocked(listen);

describe('AccountPanel', () => {
  let accountChanged: ((reason?: 'loginSucceeded' | 'loginFailed' | 'accountUpdated') => void) | undefined;
  const unlisten = vi.fn();

  beforeEach(() => {
    accountChanged = undefined;
    unlisten.mockReset();
    invokeMock.mockReset();
    listenMock.mockReset();
    listenMock.mockImplementation(async (_event, handler) => {
      accountChanged = (reason = 'accountUpdated') => handler({
        event: 'account-state-changed',
        id: 1,
        payload: { reason },
      });
      return unlisten;
    });
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it('opens ChatGPT login when the account is signed out', async () => {
    invokeMock
      .mockResolvedValueOnce({ account: { state: 'signedOut' }, loginPending: false })
      .mockResolvedValueOnce({ state: 'browserOpened' })
      .mockResolvedValueOnce({ state: 'cancelled' });

    render(<AccountPanel />);
    await userEvent.click(await screen.findByRole('button', { name: 'ChatGPT로 로그인' }));

    expect(invokeMock).toHaveBeenCalledWith('start_chatgpt_login');
    expect(await screen.findByRole('button', { name: '로그인 취소' })).toBeVisible();
  });

  it('cancels a pending login from the button', async () => {
    invokeMock
      .mockResolvedValueOnce({ account: { state: 'signedOut' }, loginPending: false })
      .mockResolvedValueOnce({ state: 'browserOpened' })
      .mockResolvedValueOnce({ state: 'cancelled' });

    render(<AccountPanel />);
    await userEvent.click(await screen.findByRole('button', { name: 'ChatGPT로 로그인' }));
    await userEvent.click(await screen.findByRole('button', { name: '로그인 취소' }));

    expect(invokeMock).toHaveBeenCalledWith('cancel_chatgpt_login');
    expect(await screen.findByRole('button', { name: 'ChatGPT로 로그인' })).toBeVisible();
  });

  it('cancels only when a panel with a pending login closes', async () => {
    invokeMock
      .mockResolvedValueOnce({ account: { state: 'signedOut' }, loginPending: false })
      .mockResolvedValueOnce({ state: 'browserOpened' })
      .mockResolvedValueOnce({ state: 'cancelled' });
    const pendingPanel = render(<AccountPanel />);
    await userEvent.click(await screen.findByRole('button', { name: 'ChatGPT로 로그인' }));
    await screen.findByRole('button', { name: '로그인 취소' });

    pendingPanel.unmount();
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('cancel_chatgpt_login'));

    invokeMock.mockReset();
    invokeMock.mockResolvedValueOnce({ account: { state: 'signedOut' }, loginPending: false });
    const idlePanel = render(<AccountPanel />);
    await screen.findByRole('button', { name: 'ChatGPT로 로그인' });
    idlePanel.unmount();
    expect(invokeMock).not.toHaveBeenCalledWith('cancel_chatgpt_login');
  });

  it('refreshes account and rate limits after a sanitized account event', async () => {
    invokeMock
      .mockResolvedValueOnce({ account: { state: 'signedOut' }, loginPending: false })
      .mockResolvedValueOnce({
        account: { state: 'signedIn', emailHint: 'a***@example.com', plan: 'plus' },
        loginPending: false,
      })
      .mockResolvedValueOnce({
        primaryUsedPercent: 25,
        primaryResetsAt: 1_730_947_200,
        secondaryUsedPercent: null,
        secondaryResetsAt: null,
      });

    render(<AccountPanel />);
    await screen.findByRole('button', { name: 'ChatGPT로 로그인' });
    expect(accountChanged).toBeDefined();
    accountChanged?.();

    expect(await screen.findByText('연결됨')).toBeVisible();
    expect(screen.getByText('사용 25%')).toBeVisible();
    expect(screen.getByText(formatResetTime(1_730_947_200))).toBeVisible();
    expect(screen.getByText('제한 정보 없음')).toBeVisible();
    expect(invokeMock).toHaveBeenCalledWith('get_account');
    expect(invokeMock).toHaveBeenCalledWith('get_rate_limits');
  });

  it('does not let an older account read overwrite a newer account event', async () => {
    let resolveOlder!: (value: { account: { state: 'signedOut' }; loginPending: false }) => void;
    const olderRead = new Promise<{ account: { state: 'signedOut' }; loginPending: false }>((resolve) => {
      resolveOlder = resolve;
    });
    invokeMock
      .mockReturnValueOnce(olderRead)
      .mockResolvedValueOnce({
        account: { state: 'signedIn', emailHint: null, plan: 'plus' },
        loginPending: false,
      })
      .mockResolvedValueOnce({
        primaryUsedPercent: null,
        primaryResetsAt: null,
        secondaryUsedPercent: null,
        secondaryResetsAt: null,
      });

    render(<AccountPanel />);
    await waitFor(() => expect(accountChanged).toBeDefined());
    accountChanged?.();
    expect(await screen.findByText('연결됨')).toBeVisible();

    await act(async () => resolveOlder({ account: { state: 'signedOut' }, loginPending: false }));

    expect(screen.getByText('연결됨')).toBeVisible();
    expect(screen.getByRole('button', { name: '다시 로그인' })).toBeVisible();
  });

  it('returns to signed out state when the browser opener fails', async () => {
    invokeMock
      .mockResolvedValueOnce({ account: { state: 'signedOut' }, loginPending: false })
      .mockRejectedValueOnce('browser_open_failed');

    render(<AccountPanel />);
    await userEvent.click(await screen.findByRole('button', { name: 'ChatGPT로 로그인' }));

    expect(await screen.findByText('브라우저를 열 수 없습니다. 다시 시도해 주세요.')).toBeVisible();
    expect(screen.getByRole('button', { name: 'ChatGPT로 로그인' })).toBeEnabled();
  });

  it.each(['browser_open_failed_login_pending', 'login_cleanup_pending'])(
    'keeps cancellation available when login cleanup must be retried (%s)',
    async (errorCode) => {
    invokeMock
      .mockResolvedValueOnce({ account: { state: 'signedOut' }, loginPending: false })
      .mockRejectedValueOnce(errorCode)
      .mockResolvedValueOnce({ state: 'cancelled' });

    render(<AccountPanel />);
    await userEvent.click(await screen.findByRole('button', { name: 'ChatGPT로 로그인' }));

    expect(await screen.findByText('로그인 요청을 정리하려면 취소해 주세요.')).toBeVisible();
    await userEvent.click(screen.getByRole('button', { name: '로그인 취소' }));
    expect(invokeMock).toHaveBeenCalledWith('cancel_chatgpt_login');
    },
  );

  it('recovers a pending login from the authoritative backend snapshot after remount', async () => {
    invokeMock.mockResolvedValueOnce({
      account: { state: 'signedOut' },
      loginPending: true,
    });

    render(<AccountPanel locale="en" />);

    expect(await screen.findByText('Complete sign-in in your browser.')).toBeVisible();
    expect(screen.getByRole('button', { name: 'Cancel sign-in' })).toBeVisible();
  });

  it('cancels a login that finishes opening after the panel was closed', async () => {
    let finishOpening!: (value: { state: 'browserOpened' }) => void;
    const opening = new Promise<{ state: 'browserOpened' }>((resolve) => {
      finishOpening = resolve;
    });
    invokeMock
      .mockResolvedValueOnce({ account: { state: 'signedOut' }, loginPending: false })
      .mockReturnValueOnce(opening)
      .mockResolvedValueOnce({ state: 'cancelled' });

    const panel = render(<AccountPanel locale="en" />);
    await userEvent.click(await screen.findByRole('button', { name: 'Sign in with ChatGPT' }));
    panel.unmount();
    await act(async () => finishOpening({ state: 'browserOpened' }));

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('cancel_chatgpt_login'));
  });

  it('refreshes instead of forcing signed out when cancellation reports not pending after success', async () => {
    let finishCancellation!: (value: { state: 'notPending' }) => void;
    const cancellation = new Promise<{ state: 'notPending' }>((resolve) => {
      finishCancellation = resolve;
    });
    invokeMock
      .mockResolvedValueOnce({ account: { state: 'signedOut' }, loginPending: false })
      .mockResolvedValueOnce({ state: 'browserOpened' })
      .mockReturnValueOnce(cancellation)
      .mockResolvedValueOnce({
        account: { state: 'signedIn', emailHint: null, plan: 'plus' },
        loginPending: false,
      })
      .mockResolvedValueOnce({
        primaryUsedPercent: null,
        primaryResetsAt: null,
        secondaryUsedPercent: null,
        secondaryResetsAt: null,
      })
      .mockResolvedValueOnce({
        account: { state: 'signedIn', emailHint: null, plan: 'plus' },
        loginPending: false,
      })
      .mockResolvedValueOnce({
        primaryUsedPercent: null,
        primaryResetsAt: null,
        secondaryUsedPercent: null,
        secondaryResetsAt: null,
      });

    render(<AccountPanel locale="en" />);
    await userEvent.click(await screen.findByRole('button', { name: 'Sign in with ChatGPT' }));
    await userEvent.click(await screen.findByRole('button', { name: 'Cancel sign-in' }));
    accountChanged?.('loginSucceeded');
    await act(async () => finishCancellation({ state: 'notPending' }));

    expect(await screen.findByText('Connected')).toBeVisible();
    expect(screen.getByRole('button', { name: 'Sign in again' })).toBeVisible();
  });

  it('shows a localized safe fallback for timestamps outside the Date range', () => {
    expect(formatResetTime(Number.POSITIVE_INFINITY, 'en')).toBe('Reset time unavailable');
    expect(formatResetTime(8_640_000_000_001, 'ko')).toBe('재설정 시간 정보 없음');
  });
});
