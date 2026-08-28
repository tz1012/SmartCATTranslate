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
  let accountChanged: (() => void) | undefined;
  const unlisten = vi.fn();

  beforeEach(() => {
    accountChanged = undefined;
    unlisten.mockReset();
    invokeMock.mockReset();
    listenMock.mockReset();
    listenMock.mockImplementation(async (_event, handler) => {
      accountChanged = () => handler({ event: 'account-state-changed', id: 1, payload: { reason: 'accountUpdated' } });
      return unlisten;
    });
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it('opens ChatGPT login when the account is signed out', async () => {
    invokeMock
      .mockResolvedValueOnce({ state: 'signedOut' })
      .mockResolvedValueOnce({ state: 'browserOpened' })
      .mockResolvedValueOnce({ state: 'cancelled' });

    render(<AccountPanel />);
    await userEvent.click(await screen.findByRole('button', { name: 'ChatGPT로 로그인' }));

    expect(invokeMock).toHaveBeenCalledWith('start_chatgpt_login');
    expect(await screen.findByRole('button', { name: '로그인 취소' })).toBeVisible();
  });

  it('cancels a pending login from the button', async () => {
    invokeMock
      .mockResolvedValueOnce({ state: 'signedOut' })
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
      .mockResolvedValueOnce({ state: 'signedOut' })
      .mockResolvedValueOnce({ state: 'browserOpened' })
      .mockResolvedValueOnce({ state: 'cancelled' });
    const pendingPanel = render(<AccountPanel />);
    await userEvent.click(await screen.findByRole('button', { name: 'ChatGPT로 로그인' }));
    await screen.findByRole('button', { name: '로그인 취소' });

    pendingPanel.unmount();
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('cancel_chatgpt_login'));

    invokeMock.mockReset();
    invokeMock.mockResolvedValueOnce({ state: 'signedOut' });
    const idlePanel = render(<AccountPanel />);
    await screen.findByRole('button', { name: 'ChatGPT로 로그인' });
    idlePanel.unmount();
    expect(invokeMock).not.toHaveBeenCalledWith('cancel_chatgpt_login');
  });

  it('refreshes account and rate limits after a sanitized account event', async () => {
    invokeMock
      .mockResolvedValueOnce({ state: 'signedOut' })
      .mockResolvedValueOnce({ state: 'signedIn', emailHint: 'a***@example.com', plan: 'plus' })
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
    let resolveOlder!: (value: { state: 'signedOut' }) => void;
    const olderRead = new Promise<{ state: 'signedOut' }>((resolve) => {
      resolveOlder = resolve;
    });
    invokeMock
      .mockReturnValueOnce(olderRead)
      .mockResolvedValueOnce({ state: 'signedIn', emailHint: null, plan: 'plus' })
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

    await act(async () => resolveOlder({ state: 'signedOut' }));

    expect(screen.getByText('연결됨')).toBeVisible();
    expect(screen.getByRole('button', { name: '다시 로그인' })).toBeVisible();
  });

  it('returns to signed out state when the browser opener fails', async () => {
    invokeMock
      .mockResolvedValueOnce({ state: 'signedOut' })
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
      .mockResolvedValueOnce({ state: 'signedOut' })
      .mockRejectedValueOnce(errorCode)
      .mockResolvedValueOnce({ state: 'cancelled' });

    render(<AccountPanel />);
    await userEvent.click(await screen.findByRole('button', { name: 'ChatGPT로 로그인' }));

    expect(await screen.findByText('로그인 요청을 정리하려면 취소해 주세요.')).toBeVisible();
    await userEvent.click(screen.getByRole('button', { name: '로그인 취소' }));
    expect(invokeMock).toHaveBeenCalledWith('cancel_chatgpt_login');
    },
  );
});
