import { useCallback, useEffect, useRef, useState } from 'react';
import {
  cancelChatgptLogin,
  getAccount,
  getRateLimits,
  onAccountStateChanged,
  startChatgptLogin,
  type AccountChangeReason,
  type AccountState,
  type RateLimitState,
} from './accountApi';

type PanelPhase = 'checking' | 'signedOut' | 'opening' | 'pending' | 'signedIn';
export type AccountPanelLocale = 'ko' | 'en';

const copy = {
  ko: {
    accountLabel: 'ChatGPT 계정',
    checking: '연결 확인 중',
    connected: '연결됨',
    pending: '브라우저에서 로그인을 완료해 주세요.',
    accountError: '계정 상태를 확인할 수 없습니다.',
    loginFailed: 'ChatGPT 로그인에 실패했습니다. 다시 시도해 주세요.',
    cleanupError: '로그인 요청을 정리하려면 취소해 주세요.',
    openError: '브라우저를 열 수 없습니다. 다시 시도해 주세요.',
    cancelError: '로그인을 취소할 수 없습니다. 다시 시도해 주세요.',
    opening: '브라우저 여는 중',
    signIn: 'ChatGPT로 로그인',
    cancel: '로그인 취소',
    signInAgain: '다시 로그인',
    primaryLimit: '기본 제한',
    secondaryLimit: '보조 제한',
    noLimit: '제한 정보 없음',
    used: (value: number) => `사용 ${value}%`,
    invalidReset: '재설정 시간 정보 없음',
    dateLocale: 'ko-KR',
  },
  en: {
    accountLabel: 'ChatGPT account',
    checking: 'Checking connection',
    connected: 'Connected',
    pending: 'Complete sign-in in your browser.',
    accountError: 'Unable to check the account status.',
    loginFailed: 'ChatGPT sign-in failed. Please try again.',
    cleanupError: 'Cancel the sign-in request before trying again.',
    openError: 'Unable to open the browser. Please try again.',
    cancelError: 'Unable to cancel sign-in. Please try again.',
    opening: 'Opening browser',
    signIn: 'Sign in with ChatGPT',
    cancel: 'Cancel sign-in',
    signInAgain: 'Sign in again',
    primaryLimit: 'Primary limit',
    secondaryLimit: 'Secondary limit',
    noLimit: 'Limit information unavailable',
    used: (value: number) => `${value}% used`,
    invalidReset: 'Reset time unavailable',
    dateLocale: 'en-US',
  },
} as const;

const emptyLimits: RateLimitState = {
  primaryUsedPercent: null,
  primaryResetsAt: null,
  secondaryUsedPercent: null,
  secondaryResetsAt: null,
};

export function formatResetTime(
  unixSeconds: number,
  locale: AccountPanelLocale = 'ko',
): string {
  const labels = copy[locale];
  if (
    !Number.isFinite(unixSeconds) ||
    unixSeconds < 0 ||
    unixSeconds > 8_640_000_000_000
  ) {
    return labels.invalidReset;
  }
  try {
    return new Intl.DateTimeFormat(labels.dateLocale, {
      dateStyle: 'medium',
      timeStyle: 'short',
    }).format(new Date(unixSeconds * 1000));
  } catch {
    return labels.invalidReset;
  }
}

export function AccountPanel({ locale = 'ko' }: { locale?: AccountPanelLocale }) {
  const labels = copy[locale];
  const [phase, setPhase] = useState<PanelPhase>('checking');
  const [account, setAccount] = useState<AccountState>({ state: 'signedOut' });
  const [limits, setLimits] = useState<RateLimitState>(emptyLimits);
  const [error, setError] = useState<string | null>(null);
  const pendingRef = useRef(false);
  const openingRef = useRef(false);
  const cancellationRequestedRef = useRef(false);
  const cancellingRef = useRef(false);
  const mountedRef = useRef(false);
  const refreshGenerationRef = useRef(0);

  const refreshAccount = useCallback(async (reason?: AccountChangeReason) => {
    const generation = ++refreshGenerationRef.current;
    try {
      const snapshot = await getAccount();
      if (!mountedRef.current || generation !== refreshGenerationRef.current) return;
      const nextAccount = snapshot.account;
      pendingRef.current = snapshot.loginPending;
      if (nextAccount.state === 'signedIn') {
        let nextLimits = emptyLimits;
        try {
          nextLimits = await getRateLimits();
        } catch {
          nextLimits = emptyLimits;
        }
        if (!mountedRef.current || generation !== refreshGenerationRef.current) return;
        setAccount(nextAccount);
        setLimits(nextLimits);
        setPhase('signedIn');
      } else {
        setAccount(nextAccount);
        setPhase(snapshot.loginPending ? 'pending' : 'signedOut');
        setLimits(emptyLimits);
      }
      setError(reason === 'loginFailed' ? labels.loginFailed : null);
    } catch {
      if (!mountedRef.current || generation !== refreshGenerationRef.current) return;
      setPhase('signedOut');
      setError(labels.accountError);
    }
  }, [labels]);

  const performCancellation = useCallback(async (showResult: boolean) => {
    if (cancellingRef.current) return;
    cancellingRef.current = true;
    try {
      const result = await cancelChatgptLogin();
      pendingRef.current = false;
      cancellationRequestedRef.current = false;
      if (!mountedRef.current || !showResult) return;
      if (result.state === 'cancelled') {
        refreshGenerationRef.current += 1;
        setAccount({ state: 'signedOut' });
        setLimits(emptyLimits);
        setPhase('signedOut');
        setError(null);
      } else {
        await refreshAccount();
      }
    } catch {
      if (mountedRef.current && showResult) {
        pendingRef.current = true;
        setPhase('pending');
        setError(labels.cancelError);
      }
    } finally {
      cancellingRef.current = false;
    }
  }, [labels, refreshAccount]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    mountedRef.current = true;
    void refreshAccount();
    try {
      void onAccountStateChanged((reason) => {
        void refreshAccount(reason);
      })
        .then((stop) => {
          if (disposed) {
            stop();
          } else {
            unlisten = stop;
          }
        })
        .catch(() => undefined);
    } catch {
      // The browser test shell has no Tauri event bridge.
    }
    return () => {
      disposed = true;
      mountedRef.current = false;
      refreshGenerationRef.current += 1;
      unlisten?.();
      if (openingRef.current) {
        cancellationRequestedRef.current = true;
      } else if (pendingRef.current) {
        void performCancellation(false);
      }
    };
  }, [performCancellation, refreshAccount]);

  const beginLogin = async () => {
    const loginGeneration = ++refreshGenerationRef.current;
    openingRef.current = true;
    cancellationRequestedRef.current = false;
    setPhase('opening');
    setError(null);
    try {
      await startChatgptLogin();
      openingRef.current = false;
      pendingRef.current = true;
      if (!mountedRef.current || cancellationRequestedRef.current) {
        await performCancellation(false);
        return;
      }
      if (loginGeneration !== refreshGenerationRef.current) return;
      setPhase('pending');
    } catch (reason) {
      openingRef.current = false;
      if (
        reason === 'browser_open_failed_login_pending' ||
        reason === 'login_cleanup_pending'
      ) {
        pendingRef.current = true;
        if (!mountedRef.current || cancellationRequestedRef.current) {
          await performCancellation(false);
          return;
        }
        setPhase('pending');
        setError(labels.cleanupError);
      } else {
        pendingRef.current = false;
        if (!mountedRef.current) return;
        setPhase('signedOut');
        setError(labels.openError);
      }
    }
  };

  const cancelLogin = async () => {
    cancellationRequestedRef.current = true;
    if (!openingRef.current) {
      await performCancellation(true);
    }
  };

  return (
    <aside className="account-panel" aria-label={labels.accountLabel}>
      <div aria-live="polite">
        {phase === 'checking' && <p>{labels.checking}</p>}
        {phase === 'signedIn' && <p>{labels.connected}</p>}
        {phase === 'pending' && <p>{labels.pending}</p>}
        {error && <p role="alert">{error}</p>}
      </div>

      {phase === 'signedIn' && account.state === 'signedIn' && (
        <div>
          {account.emailHint && <p>{account.emailHint}</p>}
          {account.plan && <p>{account.plan}</p>}
          <RateLimitWindow
            label={labels.primaryLimit}
            usedPercent={limits.primaryUsedPercent}
            resetsAt={limits.primaryResetsAt}
            locale={locale}
          />
          <RateLimitWindow
            label={labels.secondaryLimit}
            usedPercent={limits.secondaryUsedPercent}
            resetsAt={limits.secondaryResetsAt}
            locale={locale}
          />
        </div>
      )}

      {(phase === 'signedOut' || phase === 'opening') && (
        <button type="button" onClick={() => void beginLogin()} disabled={phase === 'opening'}>
          {phase === 'opening' ? labels.opening : labels.signIn}
        </button>
      )}
      {(phase === 'pending' || phase === 'opening') && (
        <button type="button" onClick={() => void cancelLogin()}>
          {labels.cancel}
        </button>
      )}
      {phase === 'signedIn' && (
        <button type="button" onClick={() => void beginLogin()}>
          {labels.signInAgain}
        </button>
      )}
    </aside>
  );
}

function RateLimitWindow({
  label,
  usedPercent,
  resetsAt,
  locale,
}: {
  label: string;
  usedPercent: number | null;
  resetsAt: number | null;
  locale: AccountPanelLocale;
}) {
  const labels = copy[locale];
  return (
    <section aria-label={label}>
      <h3>{label}</h3>
      {usedPercent === null && resetsAt === null ? (
        <p>{labels.noLimit}</p>
      ) : (
        <>
          {usedPercent !== null && <p>{labels.used(usedPercent)}</p>}
          {resetsAt !== null && <p>{formatResetTime(resetsAt, locale)}</p>}
        </>
      )}
    </section>
  );
}
