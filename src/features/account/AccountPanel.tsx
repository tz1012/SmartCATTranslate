import { useCallback, useEffect, useRef, useState } from 'react';
import {
  cancelChatgptLogin,
  getAccount,
  getRateLimits,
  onAccountStateChanged,
  startChatgptLogin,
  type AccountState,
  type RateLimitState,
} from './accountApi';

type PanelPhase = 'checking' | 'signedOut' | 'opening' | 'pending' | 'signedIn';

const emptyLimits: RateLimitState = {
  primaryUsedPercent: null,
  primaryResetsAt: null,
  secondaryUsedPercent: null,
  secondaryResetsAt: null,
};

export function formatResetTime(unixSeconds: number): string {
  return new Intl.DateTimeFormat(undefined, {
    dateStyle: 'medium',
    timeStyle: 'short',
  }).format(new Date(unixSeconds * 1000));
}

export function AccountPanel() {
  const [phase, setPhase] = useState<PanelPhase>('checking');
  const [account, setAccount] = useState<AccountState>({ state: 'signedOut' });
  const [limits, setLimits] = useState<RateLimitState>(emptyLimits);
  const [error, setError] = useState<string | null>(null);
  const pendingRef = useRef(false);
  const cancellingRef = useRef(false);
  const refreshGenerationRef = useRef(0);

  const refreshAccount = useCallback(async () => {
    const generation = ++refreshGenerationRef.current;
    try {
      const nextAccount = await getAccount();
      if (generation !== refreshGenerationRef.current) return;
      if (nextAccount.state === 'signedIn') {
        let nextLimits = emptyLimits;
        try {
          nextLimits = await getRateLimits();
        } catch {
          nextLimits = emptyLimits;
        }
        if (generation !== refreshGenerationRef.current) return;
        setAccount(nextAccount);
        setLimits(nextLimits);
        setPhase('signedIn');
      } else {
        setAccount(nextAccount);
        setPhase(pendingRef.current ? 'pending' : 'signedOut');
        setLimits(emptyLimits);
      }
      setError(null);
    } catch {
      if (generation !== refreshGenerationRef.current) return;
      setPhase('signedOut');
      setError('계정 상태를 확인할 수 없습니다.');
    }
  }, []);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void refreshAccount();
    try {
      void onAccountStateChanged(() => {
        pendingRef.current = false;
        void refreshAccount();
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
      refreshGenerationRef.current += 1;
      unlisten?.();
      if (pendingRef.current && !cancellingRef.current) {
        cancellingRef.current = true;
        void cancelChatgptLogin().catch(() => undefined);
      }
    };
  }, [refreshAccount]);

  const beginLogin = async () => {
    setPhase('opening');
    setError(null);
    try {
      await startChatgptLogin();
      pendingRef.current = true;
      setPhase('pending');
    } catch (reason) {
      if (
        reason === 'browser_open_failed_login_pending' ||
        reason === 'login_cleanup_pending'
      ) {
        pendingRef.current = true;
        setPhase('pending');
        setError('로그인 요청을 정리하려면 취소해 주세요.');
      } else {
        pendingRef.current = false;
        setPhase('signedOut');
        setError('브라우저를 열 수 없습니다. 다시 시도해 주세요.');
      }
    }
  };

  const cancelLogin = async () => {
    cancellingRef.current = true;
    try {
      await cancelChatgptLogin();
      pendingRef.current = false;
      setPhase('signedOut');
      setError(null);
    } catch {
      setError('로그인을 취소할 수 없습니다. 다시 시도해 주세요.');
    } finally {
      cancellingRef.current = false;
    }
  };

  return (
    <aside className="account-panel" aria-label="ChatGPT 계정">
      <div aria-live="polite">
        {phase === 'checking' && <p>연결 확인 중</p>}
        {phase === 'signedIn' && <p>연결됨</p>}
        {phase === 'pending' && <p>브라우저에서 로그인을 완료해 주세요.</p>}
        {error && <p role="alert">{error}</p>}
      </div>

      {phase === 'signedIn' && account.state === 'signedIn' && (
        <div>
          {account.emailHint && <p>{account.emailHint}</p>}
          {account.plan && <p>{account.plan}</p>}
          <RateLimitWindow
            label="기본 제한"
            usedPercent={limits.primaryUsedPercent}
            resetsAt={limits.primaryResetsAt}
          />
          <RateLimitWindow
            label="보조 제한"
            usedPercent={limits.secondaryUsedPercent}
            resetsAt={limits.secondaryResetsAt}
          />
        </div>
      )}

      {(phase === 'signedOut' || phase === 'opening') && (
        <button type="button" onClick={() => void beginLogin()} disabled={phase === 'opening'}>
          {phase === 'opening' ? '브라우저 여는 중' : 'ChatGPT로 로그인'}
        </button>
      )}
      {phase === 'pending' && (
        <button type="button" onClick={() => void cancelLogin()}>
          로그인 취소
        </button>
      )}
      {phase === 'signedIn' && (
        <button type="button" onClick={() => void beginLogin()}>
          다시 로그인
        </button>
      )}
    </aside>
  );
}

function RateLimitWindow({
  label,
  usedPercent,
  resetsAt,
}: {
  label: string;
  usedPercent: number | null;
  resetsAt: number | null;
}) {
  return (
    <section aria-label={label}>
      <h3>{label}</h3>
      {usedPercent === null && resetsAt === null ? (
        <p>제한 정보 없음</p>
      ) : (
        <>
          {usedPercent !== null && <p>사용 {usedPercent}%</p>}
          {resetsAt !== null && <p>{formatResetTime(resetsAt)}</p>}
        </>
      )}
    </section>
  );
}
