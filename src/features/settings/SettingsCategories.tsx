import type { AppLocale } from './SettingsView';

export type SettingsCategory = 'general' | 'translation' | 'shortcuts' | 'privacy' | 'updates';

const labels = {
  ko: {
    group: '설정 분류', general: '일반', translation: '번역', shortcuts: '단축키', privacy: '개인정보·기록', updates: '업데이트',
  },
  en: {
    group: 'Settings categories', general: 'General', translation: 'Translation', shortcuts: 'Shortcuts', privacy: 'Privacy & history', updates: 'Updates',
  },
} as const;

const categories: SettingsCategory[] = ['general', 'translation', 'shortcuts', 'privacy', 'updates'];

export function SettingsCategories({
  locale,
  value,
  onChange,
}: {
  locale: AppLocale;
  value: SettingsCategory;
  onChange: (category: SettingsCategory) => void;
}) {
  const copy = labels[locale];
  return (
    <nav className="settings-categories" role="tablist" aria-label={copy.group} aria-orientation="vertical">
      {categories.map((category) => (
        <button
          key={category}
          id={`settings-tab-${category}`}
          type="button"
          role="tab"
          aria-selected={value === category}
          aria-controls={`settings-panel-${category}`}
          onClick={() => onChange(category)}
        >
          {copy[category]}
        </button>
      ))}
    </nav>
  );
}
