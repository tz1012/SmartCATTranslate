import { useState } from 'react';
import type { AppLocale, GlossaryEntry } from './SettingsView';
import { languageLabel, SUPPORTED_LANGUAGES } from './languages';
import { createUuidV4 } from './uuid';

const copy = {
  ko: {
    group: '용어집', sourceLanguage: '원문 언어', targetLanguage: '대상 언어', sourceTerm: '원문 용어',
    targetTerm: '번역 용어', protect: '번역하지 않고 보호', add: '용어 추가', duplicate: '같은 원문 용어가 이미 있습니다',
    source: '원문', target: '번역', behavior: '동작', protected: '보호', translated: '지정 번역', remove: '삭제', required: '용어를 입력하세요',
  },
  en: {
    group: 'Glossary', sourceLanguage: 'Source language', targetLanguage: 'Target language', sourceTerm: 'Source term',
    targetTerm: 'Translated term', protect: 'Protect without translating', add: 'Add term', duplicate: 'This source term already exists',
    source: 'Source', target: 'Translation', behavior: 'Behavior', protected: 'Protected', translated: 'Specified translation', remove: 'Remove', required: 'Enter a term',
  },
} as const;

function newId() {
  return createUuidV4();
}

export function GlossaryEditor({
  locale,
  entries,
  onChange,
}: {
  locale: AppLocale;
  entries: GlossaryEntry[];
  onChange: (entries: GlossaryEntry[]) => void;
}) {
  const labels = copy[locale];
  const [sourceLanguage, setSourceLanguage] = useState('en');
  const [targetLanguage, setTargetLanguage] = useState('ko');
  const [sourceTerm, setSourceTerm] = useState('');
  const [targetTerm, setTargetTerm] = useState('');
  const [protectOnly, setProtectOnly] = useState(false);
  const [error, setError] = useState('');

  const addEntry = () => {
    const source = sourceTerm.trim();
    const target = targetTerm.trim();
    if (!source || (!protectOnly && !target)) {
      setError(labels.required);
      return;
    }
    if (entries.some((entry) => entry.sourceLanguage.toLowerCase() === sourceLanguage.toLowerCase()
      && entry.targetLanguage.toLowerCase() === targetLanguage.toLowerCase()
      && entry.sourceTerm.trim().toLowerCase() === source.toLowerCase())) {
      setError(labels.duplicate);
      return;
    }
    onChange([...entries, {
      id: newId(), sourceLanguage, targetLanguage, sourceTerm: source,
      targetTerm: protectOnly ? '' : target, protectOnly,
    }]);
    setSourceTerm('');
    setTargetTerm('');
    setError('');
  };

  return (
    <fieldset>
      <legend>{labels.group}</legend>
      <label>{labels.sourceLanguage}
        <select value={sourceLanguage} onChange={(event) => setSourceLanguage(event.target.value)}>
          {SUPPORTED_LANGUAGES.map((language) => <option key={language.code} value={language.code}>{languageLabel(language, locale)}</option>)}
        </select>
      </label>
      <label>{labels.targetLanguage}
        <select value={targetLanguage} onChange={(event) => setTargetLanguage(event.target.value)}>
          {SUPPORTED_LANGUAGES.map((language) => <option key={language.code} value={language.code}>{languageLabel(language, locale)}</option>)}
        </select>
      </label>
      <label>{labels.sourceTerm}<input value={sourceTerm} onChange={(event) => setSourceTerm(event.target.value)} /></label>
      <label>{labels.targetTerm}<input value={targetTerm} disabled={protectOnly} onChange={(event) => setTargetTerm(event.target.value)} /></label>
      <label><input type="checkbox" checked={protectOnly} onChange={(event) => setProtectOnly(event.target.checked)} />{labels.protect}</label>
      <button type="button" onClick={addEntry}>{labels.add}</button>
      {error && <p role="alert">{error}</p>}
      <table>
        <thead><tr><th>{labels.source}</th><th>{labels.target}</th><th>{labels.behavior}</th><th><span className="visually-hidden">{labels.remove}</span></th></tr></thead>
        <tbody>{entries.map((entry) => (
          <tr key={entry.id}>
            <td>{entry.sourceTerm}</td><td>{entry.targetTerm || '—'}</td><td>{entry.protectOnly ? labels.protected : labels.translated}</td>
            <td><button type="button" aria-label={`${labels.remove}: ${entry.sourceTerm}`} onClick={() => onChange(entries.filter((candidate) => candidate.id !== entry.id))}>{labels.remove}</button></td>
          </tr>
        ))}</tbody>
      </table>
    </fieldset>
  );
}
