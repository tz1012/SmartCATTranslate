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
import { AppNotificationPopover, type AppNotification } from './AppNotificationPopover';
import type { CompletedTextTranslation } from '../lib/types';
import { installSignedUpdate } from '../features/settings/installUpdate';

export type AppLocale = 'ko' | 'en';
type PrivacyStatus = { cleanupPending: boolean; retentionPending: boolean };
type UpdateCheckResult = { available: boolean; version: string | null; consentToken?: string; manualOnly?: boolean; releaseUrl?: string };

export function App({ locale }: { locale?: AppLocale }) {
  const [savedLocale, setSavedLocale] = useState<AppLocale>('ko');
  const [savedTheme, setSavedTheme] = useState<Theme>('light');
  const [activeView, setActiveView] = useState<AppView>('translate');
  const [menuOpen, setMenuOpen] = useState(false);
  const [notificationsOpen, setNotificationsOpen] = useState(false);
  const [dismissedNotificationIds, setDismissedNotificationIds] = useState<Set<string>>(() => new Set());
  const [mountedViews, setMountedViews] = useState<Set<AppView>>(() => new Set(['translate']));
  const [settingsDestination, setSettingsDestination] = useState<SettingsDestination>('general');
  const menuButtonRef = useRef<HTMLButtonElement>(null);
  const [recovery, setRecovery] = useState<PreparedDocumentRecovery>();
  const [privacyStatus, setPrivacyStatus] = useState<PrivacyStatus>();
  const [availableUpdate, setAvailableUpdate] = useState<UpdateCheckResult>();
  const [updateBusy, setUpdateBusy] = useState(false);
  const [updateStatus, setUpdateStatus] = useState('');
  const startupUpdateCheck = useRef<Promise<UpdateCheckResult> | null>(null);
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
    startupUpdateCheck.current ??= invoke<UpdateCheckResult>('check_for_update');
    void startupUpdateCheck.current
      .then((result) => {
        if (!disposed && result.available && result.version) setAvailableUpdate(result);
      })
      .catch(() => undefined);
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
    ? { navigation: '주요 화면', translate: '텍스트', capture: '이미지·화면', documents: '문서', history:'기록', settings: '설정', openMenu: '메뉴 열기', closeMenu: '메뉴 닫기', accountMenu: '계정 메뉴', notificationPanel: '알림', notifications: (count: number) => `알림 ${count}개`, dismissNotification: '확인', cleanupNotice: '임시 파일 정리가 보류되었습니다. 앱이 시작될 때 안전하게 다시 시도합니다.', retentionNotice: '기록 보관 설정을 확인하는 중입니다. 확인 전에는 기록을 자동 삭제하지 않습니다.', updateNotice: (version: string) => `새 버전 ${version}를 사용할 수 있습니다.`, updateAction: '업데이트', releaseAction: '릴리스 페이지 열기', updating: '다운로드 및 서명 확인 후 설치 중…', updateFailed: '업데이트 설치에 실패했습니다. 다시 시도해 주세요.', releaseUnavailable: '이 릴리스는 안전하게 열 수 없습니다.', settingsLocked: '번역 또는 취소 처리 중에는 설정을 열 수 없습니다.' }
    : { navigation: 'Main views', translate: 'Text', capture: 'Image & screen', documents: 'Documents', history:'History', settings: 'Settings', openMenu: 'Open menu', closeMenu: 'Close menu', accountMenu: 'Account menu', notificationPanel: 'Notifications', notifications: (count: number) => `${count} notifications`, dismissNotification: 'Dismiss', cleanupNotice: 'Temporary-file cleanup is pending and will be retried safely at startup.', retentionNotice: 'History retention is being verified. No history is automatically deleted until then.', updateNotice: (version: string) => `Version ${version} is available.`, updateAction: 'Update', releaseAction: 'Open release page', updating: 'Downloading, verifying, and installing…', updateFailed: 'The update could not be installed. Try again.', releaseUnavailable: 'This release cannot be opened safely.', settingsLocked: 'Settings are unavailable while translation or cancellation is in progress.' };

  const installAvailableUpdate = async () => {
    if (!availableUpdate?.version) return;
    if (availableUpdate.manualOnly && availableUpdate.releaseUrl) {
      await invoke('open_update_release', { url: availableUpdate.releaseUrl }).catch(() => setUpdateStatus(labels.updateFailed));
      return;
    }
    if (!availableUpdate.consentToken) return;
    setUpdateBusy(true);
    setUpdateStatus(labels.updating);
    try {
      await installSignedUpdate({ version: availableUpdate.version, consentToken: availableUpdate.consentToken });
    } catch {
      setUpdateStatus(labels.updateFailed);
      try {
        const refreshed = await invoke<UpdateCheckResult>('check_for_update');
        setAvailableUpdate(refreshed.available && refreshed.version ? refreshed : undefined);
      } catch {
        setAvailableUpdate(undefined);
      }
      setUpdateBusy(false);
    }
  };
  const canOpenManualRelease = Boolean(availableUpdate?.manualOnly && availableUpdate.releaseUrl);
  const canInstallSignedUpdate = Boolean(availableUpdate && !availableUpdate.manualOnly && availableUpdate.consentToken);
  const notifications: AppNotification[] = [
    ...(privacyStatus?.cleanupPending ? [{ id: 'cleanup', message: labels.cleanupNotice }] : []),
    ...(privacyStatus?.retentionPending ? [{ id: 'retention', message: labels.retentionNotice }] : []),
    ...(availableUpdate?.version ? [{
      id: `update-${availableUpdate.version}`,
      message: labels.updateNotice(availableUpdate.version),
      actionLabel: canOpenManualRelease ? labels.releaseAction : canInstallSignedUpdate ? labels.updateAction : undefined,
      actionDisabled: updateBusy,
      onAction: canOpenManualRelease || canInstallSignedUpdate ? () => void installAvailableUpdate() : undefined,
      status: availableUpdate.manualOnly && !availableUpdate.releaseUrl ? labels.releaseUnavailable : updateStatus,
    }] : []),
  ].filter((notification) => !dismissedNotificationIds.has(notification.id));

  const closeMenu = useCallback(() => {
    menuButtonRef.current?.focus();
    setMenuOpen(false);
  }, []);
  const navigate = useCallback((view: AppView, destination?: SettingsDestination) => {
    if (view === 'settings' && translationActive) return;
    if (destination) setSettingsDestination(destination);
    if (view === 'translate' || view === 'capture' || view === 'documents') {
      setMountedViews((current) => current.has(view) ? current : new Set(current).add(view));
    }
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
          statusMessage={translationActive ? labels.settingsLocked : undefined}
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
          <AppNotificationPopover
            label={labels.notificationPanel}
            dismissLabel={labels.dismissNotification}
            notifications={notifications}
            onClose={() => setNotificationsOpen(false)}
            onDismiss={(id) => {
              setDismissedNotificationIds((current) => new Set(current).add(id));
              if (notifications.length === 1) setNotificationsOpen(false);
            }}
          />
        )}
      </div>
      <RecoveryPrompt
        locale={accountLocale}
        onContinue={(value) => {
          setRecovery(value);
          setMountedViews((current) => current.has('documents') ? current : new Set(current).add('documents'));
          setActiveView('documents');
        }}
      />
      {mountedViews.has('translate') && (
        <div id="app-panel-translate" role="tabpanel" aria-labelledby="app-tab-translate" hidden={activeView !== 'translate'}>
          <TextWorkspace
            locale={locale}
            importedTranslation={importedTranslation}
            onImportedTranslationConsumed={consumeImportedTranslation}
            onPreferencesLoaded={acceptPreferences}
            onActivityChange={setTranslationActive}
          />
        </div>
      )}
      {mountedViews.has('capture') && (
        <div id="app-panel-capture" role="tabpanel" aria-labelledby="app-tab-capture" hidden={activeView !== 'capture'}>
          <CaptureWorkspace locale={accountLocale} onTranslationComplete={acceptCaptureTranslation} />
        </div>
      )}
      {mountedViews.has('documents') && (
        <div id="app-panel-documents" role="tabpanel" aria-labelledby="app-tab-documents" hidden={activeView !== 'documents'}>
          <DocumentWorkspace locale={accountLocale} recovery={recovery} onRecoveryConsumed={()=>setRecovery(undefined)} active={activeView === 'documents'} />
        </div>
      )}
      {activeView === 'history' ? (
        <div id="app-panel-history" role="tabpanel" aria-labelledby="app-tab-history"><HistoryView locale={accountLocale}/></div>
      ) : activeView === 'settings' ? (
        <div id="app-panel-settings" role="tabpanel" aria-label={labels.settings}>
          <SettingsView locale={locale} initialCategory={settingsDestination} onPreferencesLoaded={acceptPreferences} onPreferencesSaved={acceptPreferences} />
        </div>
      ) : null}
    </main>
  );
}
