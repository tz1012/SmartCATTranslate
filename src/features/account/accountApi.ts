import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

export type AccountState =
  | { state: 'signedOut' }
  | { state: 'signedIn'; emailHint: string | null; plan: string | null };

export type AccountSnapshot = {
  account: AccountState;
  loginPending: boolean;
};

export type AccountChangeReason = 'loginSucceeded' | 'loginFailed' | 'accountUpdated';

export type RateLimitState = {
  primaryUsedPercent: number | null;
  primaryResetsAt: number | null;
  secondaryUsedPercent: number | null;
  secondaryResetsAt: number | null;
};

export function getAccount(): Promise<AccountSnapshot> {
  return invoke<AccountSnapshot>('get_account');
}

export function getRateLimits(): Promise<RateLimitState> {
  return invoke<RateLimitState>('get_rate_limits');
}

export function startChatgptLogin(): Promise<{ state: 'browserOpened' }> {
  return invoke('start_chatgpt_login');
}

export function cancelChatgptLogin(): Promise<{ state: 'cancelled' | 'notPending' }> {
  return invoke('cancel_chatgpt_login');
}

export function onAccountStateChanged(
  handler: (reason: AccountChangeReason) => void,
): Promise<UnlistenFn> {
  return listen<{ reason: AccountChangeReason }>('account-state-changed', (event) => {
    handler(event.payload.reason);
  });
}
