import { AccountPanel } from '../features/account/AccountPanel';

export type AppLocale = 'ko' | 'en';

const copy = {
  ko: {
    workspace: '텍스트 번역',
    source: '원문',
    translation: '번역문',
  },
  en: {
    workspace: 'Text translation',
    source: 'Source text',
    translation: 'Translation',
  },
} as const;

export function App({ locale = 'ko' }: { locale?: AppLocale }) {
  const labels = copy[locale];

  return (
    <main>
      <header>
        <h1>SmartCAT Translate</h1>
      </header>
      <AccountPanel />
      <section className="translation-grid" aria-label={labels.workspace}>
        <label>
          {labels.source}
          <textarea aria-label={labels.source} />
        </label>
        <label>
          {labels.translation}
          <textarea aria-label={labels.translation} readOnly />
        </label>
      </section>
    </main>
  );
}
