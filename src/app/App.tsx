import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { SettingsView, type Theme } from '../features/settings/SettingsView';
import { TextWorkspace } from '../features/translation/TextWorkspace';
import { CaptureWorkspace } from '../features/capture/CaptureWorkspace';
import { DocumentWorkspace } from '../features/documents/DocumentWorkspace';
import { HistoryView } from '../features/history/HistoryView';
import { RecoveryPrompt } from '../features/history/RecoveryPrompt';
import type { PreparedDocumentRecovery } from '../features/history/historyApi';
import { AppTopBar, type AppView } from './AppTopBar';
import { AppMenuOverlay, type SettingsDestination } from './AppMenuOverlay';
import { AppNotificationPopover } from './AppNotificationPopover';
import type { CompletedTextTranslation } from '../lib/types';

export type AppLocale = 'ko' | 'en';
type PrivacyStatus = { cleanupPending: boolean; retentionPending: boolean };

export function App({ locale }: { locale?: AppLocale }) {
  const [savedLocale, setSavedLocale] = useState<AppLocale>('ko');
  const [savedTheme, setSavedTheme] = useState<Theme>('light');
  const [activeView, setActiveView] = useState<AppView>('translate');
  const [menuOpen, setMenuOpen] = useState(false);
  const [notificationsOpen, setNotificationsOpen] = useState(false);
  const [settingsDestination, setSettingsDestination] = useState<SettingsDestination>('general');
  const menuButtonRef = useRef<HTMLButtonElement>(null);
  const [recovery, setRecovery] = useState<PreparedDocumentRecovery>();
  const [privacyStatus, setPrivacyStatus] = useState<PrivacyStatus>();
  const [translationActive, setTranslationActive] = useState(false);
  const [importedTranslation, setImportedTranslation] = useState<CompletedTextTranslation>();
  const accountLocale = locale ?? savedLocale;
  useEffect(() => {
    let disposed = false;
    let retry: number | undefined;
    const delay = (milliseconds: number) => new Promise<void>((resolve) => window.setTimeout(resolve, milliseconds));
    const verifyInitialState = async () => {
      try {
        await Promise.all([
          invoke('get_lifecycle_status'),
          invoke('get_account'),
          invoke('get_settings'),
          invoke('get_privacy_status'),
          invoke('list_history', { limit: 1, cursor: null }),
          invoke('list_recoverable_jobs'),
        ]);
        await delay(1_500);
        if (!disposed) await invoke<boolean>('mark_app_healthy');
      } catch {
        if (!disposed) retry = window.setTimeout(() => void verifyInitialState(), 2_000);
      }
    };
    void verifyInitialState();
    return () => {
      disposed = true;
      if (retry !== undefined) window.clearTimeout(retry);
    };
  }, []);
  useEffect(() => {
    let disposed = false;
    let stopSettings: (() => void) | undefined;
    let stopPrivacy: (() => void) | undefined;
    void listen('open-settings', () => {
      setMenuOpen(false);
      setActiveView('settings');
    }).then((unlisten) => {
      if (disposed) unlisten();
      else stopSettings = unlisten;
    });
    void invoke<PrivacyStatus>('get_privacy_status').then(setPrivacyStatus).catch(() => undefined);
    void listen<PrivacyStatus>('privacy-status', (event) => setPrivacyStatus(event.payload)).then((unlisten) => {
      if (disposed) unlisten();
      else stopPrivacy = unlisten;
    });
    return () => {
      disposed = true;
      stopSettings?.();
      stopPrivacy?.();
    };
  }, []);
  const acceptPreferences = useCallback((loadedLocale: AppLocale, loadedTheme: Theme) => {
    if (locale === undefined) setSavedLocale(loadedLocale);
    setSavedTheme(loadedTheme);
  }, [locale]);
  const labels = accountLocale === 'ko'
    ? { navigation: '주요 화면', translate: '텍스트', capture: '이미지·화면', documents: '문서', history:'기록', settings: '설정', openMenu: '메뉴 열기', closeMenu: '메뉴 닫기', accountMenu: '계정 메뉴', notificationPanel: '알림', notifications: (count: number) => `알림 ${count}개`, cleanupNotice: '임시 파일 정리가 보류되었습니다. 앱이 시작될 때 안전하게 다시 시도합니다.', retentionNotice: '기록 보관 설정을 확인하는 중입니다. 확인 전에는 기록을 자동 삭제하지 않습니다.', settingsLocked: '번역 또는 취소 처리 중에는 설정을 열 수 없습니다.' }
    : { navigation: 'Main views', translate: 'Text', capture: 'Image & screen', documents: 'Documents', history:'History', settings: 'Settings', openMenu: 'Open menu', closeMenu: 'Close menu', accountMenu: 'Account menu', notificationPanel: 'Notifications', notifications: (count: number) => `${count} notifications`, cleanupNotice: 'Temporary-file cleanup is pending and will be retried safely at startup.', retentionNotice: 'History retention is being verified. No history is automatically deleted until then.', settingsLocked: 'Settings are unavailable while translation or cancellation is in progress.' };

  const notifications = [
    ...(privacyStatus?.cleanupPending ? [labels.cleanupNotice] : []),
    ...(privacyStatus?.retentionPending ? [labels.retentionNotice] : []),
  ];

  const closeMenu = useCallback(() => {
    menuButtonRef.current?.focus();
    setMenuOpen(false);
  }, []);
  const navigate = useCallback((view: AppView, destination?: SettingsDestination) => {
    if (view === 'settings' && translationActive) return;
    if (destination) setSettingsDestination(destination);
    setActiveView(view);
  }, [translationActive]);
  const consumeImportedTranslation = useCallback(() => setImportedTranslation(undefined), []);
  const acceptCaptureTranslation = useCallback((translation: CompletedTextTranslation) => {
    setImportedTranslation(translation);
    setActiveView('translate');
  }, []);

  return (
    <main data-theme={savedTheme}>
      <div className="app-shell-navigation">
        <AppTopBar
          activeView={activeView}
          labels={labels}
          menuOpen={menuOpen}
          menuButtonRef={menuButtonRef}
          notificationCount={notifications.length}
          notificationsOpen={notificationsOpen}
          onNavigate={navigate}
          onToggleNotifications={() => {
            setMenuOpen(false);
            setNotificationsOpen((open) => !open);
          }}
          onToggleMenu={() => {
            setNotificationsOpen(false);
            setMenuOpen((open) => !open);
          }}
        />
        {menuOpen && (
          <AppMenuOverlay
            locale={accountLocale}
            settingsLocked={translationActive}
            onClose={closeMenu}
            onNavigate={navigate}
          />
        )}
        {notificationsOpen && notifications.length > 0 && (
          <AppNotificationPopover label={labels.notificationPanel} notifications={notifications} onClose={() => setNotificationsOpen(false)} />
        )}
      </div>
      <RecoveryPrompt
        locale={accountLocale}
        onContinue={(value) => {
          setRecovery(value);
          setActiveView('documents');
        }}
      />
      {translationActive && <p id="settings-navigation-status" className="navigation-note" role="status">{labels.settingsLocked}</p>}
      {activeView === 'translate' ? (
        <div id="app-panel-translate" role="tabpanel" aria-labelledby="app-tab-translate">
          <TextWorkspace
            locale={locale}
            importedTranslation={importedTranslation}
            onImportedTranslationConsumed={consumeImportedTranslation}
            onPreferencesLoaded={acceptPreferences}
            onActivityChange={setTranslationActive}
          />
        </div>
      ) : activeView === 'capture' ? (
        <div id="app-panel-capture" role="tabpanel" aria-labelledby="app-tab-capture">
          <CaptureWorkspace locale={accountLocale} onTranslationComplete={acceptCaptureTranslation} />
        </div>
      ) : activeView === 'documents' ? (
        <div id="app-panel-documents" role="tabpanel" aria-labelledby="app-tab-documents">
          <DocumentWorkspace locale={accountLocale} recovery={recovery} onRecoveryConsumed={()=>setRecovery(undefined)} />
        </div>
      ) : activeView === 'history' ? (
        <div id="app-panel-history" role="tabpanel" aria-labelledby="app-tab-history"><HistoryView locale={accountLocale}/></div>
      ) : (
        <div id="app-panel-settings" role="tabpanel" aria-label={labels.settings}>
          <SettingsView locale={locale} initialCategory={settingsDestination} onPreferencesLoaded={acceptPreferences} onPreferencesSaved={acceptPreferences} />
        </div>
      )}
    </main>
  );
}
