import { useEffect, useRef, useState } from 'react';
import { getVersion } from '@tauri-apps/api/app';
import { invoke } from '@tauri-apps/api/core';
import { AccountPanel } from '../features/account/AccountPanel';
import type { AppLocale } from './App';
import type { AppView } from './AppTopBar';

const copy = {
  ko: {
    menu: '앱 메뉴', general: '일반 설정', translation: '번역 설정', hotkeys: '단축키',
    privacy: '개인정보·기록', updates: '업데이트', about: '앱 정보', quit: '종료',
    accountDetails: 'ChatGPT 계정 정보',
    product: 'SmartCAT Translate', versionUnavailable: '버전을 확인할 수 없습니다.', settingsLocked: '번역 중에는 설정을 열 수 없습니다.',
  },
  en: {
    menu: 'App menu', general: 'General settings', translation: 'Translation settings', hotkeys: 'Shortcuts',
    privacy: 'Privacy & history', updates: 'Updates', about: 'About', quit: 'Quit',
    accountDetails: 'ChatGPT account details',
    product: 'SmartCAT Translate', versionUnavailable: 'Version unavailable', settingsLocked: 'Settings are unavailable while translating.',
  },
} as const;

export type SettingsDestination = 'general' | 'translation' | 'shortcuts' | 'privacy' | 'updates';

export function AppMenuOverlay({
  locale,
  settingsLocked,
  onClose,
  onNavigate,
}: {
  locale: AppLocale;
  settingsLocked: boolean;
  onClose: () => void;
  onNavigate: (view: AppView, settingsDestination?: SettingsDestination) => void;
}) {
  const labels = copy[locale];
  const panelRef = useRef<HTMLElement>(null);
  const accountAutoHideTimer = useRef<number | undefined>(undefined);
  const [showAbout, setShowAbout] = useState(false);
  const [showAccount, setShowAccount] = useState(true);
  const [version, setVersion] = useState<
    { status: 'loading' } | { status: 'success'; value: string } | { status: 'failed' }
  >({ status: 'loading' });

  useEffect(() => {
    accountAutoHideTimer.current = window.setTimeout(() => setShowAccount(false), 3_500);
    return () => {
      if (accountAutoHideTimer.current !== undefined) window.clearTimeout(accountAutoHideTimer.current);
    };
  }, []);

  const revealAccount = () => {
    if (accountAutoHideTimer.current !== undefined) {
      window.clearTimeout(accountAutoHideTimer.current);
      accountAutoHideTimer.current = undefined;
    }
    setShowAccount(true);
  };

  useEffect(() => {
    let mounted = true;
    void getVersion().then(
      (installedVersion) => {
        if (mounted) setVersion({ status: 'success', value: installedVersion });
      },
      () => {
        if (mounted) setVersion({ status: 'failed' });
      },
    );
    return () => {
      mounted = false;
    };
  }, []);

  useEffect(() => {
    panelRef.current?.focus({ preventScroll: true });
    const handleKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.preventDefault();
        onClose();
        return;
      }
      if (event.key !== 'Tab' || !panelRef.current) return;
      const focusable = Array.from(panelRef.current.querySelectorAll<HTMLElement>('button:not(:disabled), [href], select, input'));
      if (focusable.length === 0) return;
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last.focus(); }
      else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus(); }
    };
    const handleOutside = (event: PointerEvent) => {
      const target = event.target;
      if (!(target instanceof Element)) return;
      if (!panelRef.current?.contains(target) && !target.closest('[aria-controls="app-menu-overlay"]')) onClose();
    };
    document.addEventListener('keydown', handleKey, true);
    document.addEventListener('pointerdown', handleOutside, true);
    return () => {
      document.removeEventListener('keydown', handleKey, true);
      document.removeEventListener('pointerdown', handleOutside, true);
    };
  }, [onClose]);

  const openSettings = (destination: SettingsDestination) => {
    if (settingsLocked) return;
    onNavigate('settings', destination);
    onClose();
  };

  return (
    <aside
      ref={panelRef}
      id="app-menu-overlay"
      className="app-menu-overlay"
      role="dialog"
      aria-label={labels.menu}
      tabIndex={-1}
    >
      <div
        className="app-account-anchor"
        onMouseEnter={revealAccount}
        onMouseLeave={() => setShowAccount(false)}
        onFocusCapture={revealAccount}
        onBlurCapture={(event) => {
          if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setShowAccount(false);
        }}
      >
        <button
          type="button"
          className="app-account-icon"
          aria-label={labels.accountDetails}
          aria-expanded={showAccount}
          onClick={revealAccount}
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <circle cx="12" cy="8" r="3.25" />
            <path d="M5.75 19c.55-3.2 2.65-5 6.25-5s5.7 1.8 6.25 5" />
          </svg>
        </button>
        <div className="app-account-popover" hidden={!showAccount}>
          <AccountPanel locale={locale} />
        </div>
      </div>
      <nav aria-label={labels.menu}>
        <button type="button" disabled={settingsLocked} onClick={() => openSettings('general')}><i aria-hidden="true">⚙</i><span>{labels.general}</span></button>
        <button type="button" disabled={settingsLocked} onClick={() => openSettings('translation')}><i aria-hidden="true">⌁</i><span>{labels.translation}</span></button>
        <button type="button" disabled={settingsLocked} onClick={() => openSettings('shortcuts')}><i aria-hidden="true">⌨</i><span>{labels.hotkeys}</span></button>
        <button type="button" disabled={settingsLocked} onClick={() => openSettings('privacy')}><i aria-hidden="true">♢</i><span>{labels.privacy}</span></button>
        <button type="button" disabled={settingsLocked} onClick={() => openSettings('updates')}><i aria-hidden="true">↻</i><span>{labels.updates}</span></button>
      </nav>
      {settingsLocked && <p className="app-menu-note" role="status">{labels.settingsLocked}</p>}
      <div className="app-menu-secondary">
        <button type="button" aria-expanded={showAbout} onClick={() => setShowAbout((visible) => !visible)}><i aria-hidden="true">ⓘ</i><span>{labels.about}</span></button>
        {showAbout && <p>
          {labels.product}
          {version.status === 'success' ? ` ${version.value}` : version.status === 'failed' ? <> — <span>{labels.versionUnavailable}</span></> : ''}
        </p>}
        <button type="button" onClick={() => void invoke('quit_application')}><i aria-hidden="true">⇥</i><span>{labels.quit}</span></button>
      </div>
    </aside>
  );
}
