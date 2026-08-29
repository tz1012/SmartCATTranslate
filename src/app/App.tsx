import { useCallback, useState } from 'react';
import { AccountPanel } from '../features/account/AccountPanel';
import { TextWorkspace } from '../features/translation/TextWorkspace';

export type AppLocale = 'ko' | 'en';

export function App({ locale }: { locale?: AppLocale }) {
  const [savedLocale, setSavedLocale] = useState<AppLocale>('ko');
  const accountLocale = locale ?? savedLocale;
  const acceptSavedLocale = useCallback((loadedLocale: AppLocale) => {
    if (locale === undefined) setSavedLocale(loadedLocale);
  }, [locale]);

  return (
    <main>
      <header className="app-header">
        <h1>SmartCAT Translate</h1>
      </header>
      <AccountPanel locale={accountLocale} />
      <TextWorkspace locale={locale} onLocaleLoaded={acceptSavedLocale} />
    </main>
  );
}
