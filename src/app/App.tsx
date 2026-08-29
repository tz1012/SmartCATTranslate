import { useCallback, useState } from 'react';
import { AccountPanel } from '../features/account/AccountPanel';
import { SettingsView, type Theme } from '../features/settings/SettingsView';
import { TextWorkspace } from '../features/translation/TextWorkspace';

export type AppLocale = 'ko' | 'en';

export function App({ locale }: { locale?: AppLocale }) {
  const [savedLocale, setSavedLocale] = useState<AppLocale>('ko');
  const [savedTheme, setSavedTheme] = useState<Theme>('system');
  const [activeView, setActiveView] = useState<'translate' | 'settings'>('translate');
  const accountLocale = locale ?? savedLocale;
  const acceptPreferences = useCallback((loadedLocale: AppLocale, loadedTheme: Theme) => {
    if (locale === undefined) setSavedLocale(loadedLocale);
    setSavedTheme(loadedTheme);
  }, [locale]);
  const labels = accountLocale === 'ko'
    ? { navigation: '주요 화면', translate: '번역', settings: '설정' }
    : { navigation: 'Main views', translate: 'Translate', settings: 'Settings' };

  return (
    <main data-theme={savedTheme}>
      <header className="app-header">
        <h1>SmartCAT Translate</h1>
      </header>
      <AccountPanel locale={accountLocale} />
      <nav className="app-navigation" role="tablist" aria-label={labels.navigation}>
        <button id="app-tab-translate" type="button" role="tab" aria-selected={activeView === 'translate'} aria-controls="app-panel-translate" onClick={() => setActiveView('translate')}>{labels.translate}</button>
        <button id="app-tab-settings" type="button" role="tab" aria-selected={activeView === 'settings'} aria-controls="app-panel-settings" onClick={() => setActiveView('settings')}>{labels.settings}</button>
      </nav>
      {activeView === 'translate' ? (
        <div id="app-panel-translate" role="tabpanel" aria-labelledby="app-tab-translate">
          <TextWorkspace locale={locale} onPreferencesLoaded={acceptPreferences} />
        </div>
      ) : (
        <div id="app-panel-settings" role="tabpanel" aria-labelledby="app-tab-settings">
          <SettingsView locale={locale} onPreferencesLoaded={acceptPreferences} onPreferencesSaved={acceptPreferences} />
        </div>
      )}
    </main>
  );
}
