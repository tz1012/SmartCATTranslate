import { useCallback, useEffect, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { AccountPanel } from '../features/account/AccountPanel';
import { SettingsView, type Theme } from '../features/settings/SettingsView';
import { TextWorkspace } from '../features/translation/TextWorkspace';
import { CaptureWorkspace } from '../features/capture/CaptureWorkspace';
import { DocumentWorkspace } from '../features/documents/DocumentWorkspace';
import { HistoryView } from '../features/history/HistoryView';
import { RecoveryPrompt } from '../features/history/RecoveryPrompt';
import type { PreparedDocumentRecovery } from '../features/history/historyApi';

export type AppLocale = 'ko' | 'en';

export function App({ locale }: { locale?: AppLocale }) {
  const [savedLocale, setSavedLocale] = useState<AppLocale>('ko');
  const [savedTheme, setSavedTheme] = useState<Theme>('system');
  const [activeView, setActiveView] = useState<'translate' | 'capture' | 'documents' | 'history' | 'settings'>('translate');
  const [recovery,setRecovery]=useState<PreparedDocumentRecovery>();
  const [translationActive, setTranslationActive] = useState(false);
  const accountLocale = locale ?? savedLocale;
  useEffect(() => {
    let disposed = false;
    let stop: (() => void) | undefined;
    void listen('open-settings', () => setActiveView('settings')).then((unlisten) => {
      if (disposed) unlisten(); else stop = unlisten;
    });
    return () => { disposed = true; stop?.(); };
  }, []);
  const acceptPreferences = useCallback((loadedLocale: AppLocale, loadedTheme: Theme) => {
    if (locale === undefined) setSavedLocale(loadedLocale);
    setSavedTheme(loadedTheme);
  }, [locale]);
  const labels = accountLocale === 'ko'
    ? { navigation: '주요 화면', translate: '텍스트', capture: '이미지·화면', documents: '문서', history:'기록', settings: '설정', settingsLocked: '번역 또는 취소 처리 중에는 설정을 열 수 없습니다.' }
    : { navigation: 'Main views', translate: 'Text', capture: 'Image & screen', documents: 'Documents', history:'History', settings: 'Settings', settingsLocked: 'Settings are unavailable while translation or cancellation is in progress.' };

  return (
    <main data-theme={savedTheme}>
      <header className="app-header">
        <h1>SmartCAT Translate</h1>
      </header>
      <AccountPanel locale={accountLocale} />
      <RecoveryPrompt locale={accountLocale} onContinue={(value)=>{setRecovery(value);setActiveView('documents');}}/>
      <nav className="app-navigation" role="tablist" aria-label={labels.navigation} aria-busy={translationActive}>
        <button id="app-tab-translate" type="button" role="tab" aria-selected={activeView === 'translate'} aria-controls="app-panel-translate" onClick={() => setActiveView('translate')}>{labels.translate}</button>
        <button id="app-tab-capture" type="button" role="tab" aria-selected={activeView === 'capture'} aria-controls="app-panel-capture" onClick={() => setActiveView('capture')}>{labels.capture}</button>
        <button id="app-tab-documents" type="button" role="tab" aria-selected={activeView === 'documents'} aria-controls="app-panel-documents" onClick={() => setActiveView('documents')}>{labels.documents}</button>
        <button id="app-tab-history" type="button" role="tab" aria-selected={activeView === 'history'} aria-controls="app-panel-history" onClick={() => setActiveView('history')}>{labels.history}</button>
        <button id="app-tab-settings" type="button" role="tab" aria-selected={activeView === 'settings'} aria-controls="app-panel-settings" aria-describedby={translationActive ? 'settings-navigation-status' : undefined} disabled={translationActive} onClick={() => {
          if (!translationActive) setActiveView('settings');
        }}>{labels.settings}</button>
      </nav>
      {translationActive && <p id="settings-navigation-status" className="navigation-note" role="status">{labels.settingsLocked}</p>}
      {activeView === 'translate' ? (
        <div id="app-panel-translate" role="tabpanel" aria-labelledby="app-tab-translate">
          <TextWorkspace locale={locale} onPreferencesLoaded={acceptPreferences} onActivityChange={setTranslationActive} />
        </div>
      ) : activeView === 'capture' ? (
        <div id="app-panel-capture" role="tabpanel" aria-labelledby="app-tab-capture">
          <CaptureWorkspace locale={accountLocale} />
        </div>
      ) : activeView === 'documents' ? (
        <div id="app-panel-documents" role="tabpanel" aria-labelledby="app-tab-documents">
          <DocumentWorkspace locale={accountLocale} recovery={recovery} onRecoveryConsumed={()=>setRecovery(undefined)} />
        </div>
      ) : activeView === 'history' ? (
        <div id="app-panel-history" role="tabpanel" aria-labelledby="app-tab-history"><HistoryView locale={accountLocale}/></div>
      ) : (
        <div id="app-panel-settings" role="tabpanel" aria-labelledby="app-tab-settings">
          <SettingsView locale={locale} onPreferencesLoaded={acceptPreferences} onPreferencesSaved={acceptPreferences} />
        </div>
      )}
    </main>
  );
}
