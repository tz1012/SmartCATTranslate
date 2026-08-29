import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { TranslationMode, TranslationProfile } from '../../lib/types';
import { getAccount, onAccountStateChanged } from '../account/accountApi';
import type { AppLocale, AppSettings } from '../settings/SettingsView';
import { resolveDefaultProfile } from '../settings/defaultProfile';
import { languageLabel, SUPPORTED_LANGUAGES } from '../settings/languages';
import { useTranslationJob } from './useTranslationJob';

const MAX_SOURCE_CHARS = 200_000;
const MAX_SOURCE_BYTES = 1_000_000;

const copy = {
  ko: {
    workspace: '텍스트 번역', text: '텍스트', image: '이미지', document: '문서', capture: '화면 캡처', history: '기록',
    sourceLanguage: '원문 언어', targetLanguage: '대상 언어', auto: '언어 감지', swap: '언어 바꾸기', source: '원문', result: '번역문',
    translate: '번역', cancel: '취소', retry: '다시 시도', copyResult: '번역문 복사', saveResult: '번역문 저장', clear: '모두 지우기',
    signedIn: 'ChatGPT 계정 연결됨', signedOut: '번역하려면 ChatGPT 계정을 연결해 주세요.', checking: '계정과 설정을 확인하는 중입니다.',
    shortcut: '단축키: 설정되지 않음', ready: '번역할 준비가 되었습니다.', running: '번역 중입니다.', completed: '번역이 완료되었습니다.', copied: '번역문을 복사했습니다.', saved: '번역문 파일을 저장했습니다.',
    empty: '번역할 원문을 입력해 주세요.', tooLarge: '원문은 200,000자와 1,000,000바이트 이하여야 합니다.', loadError: '번역 설정을 불러올 수 없습니다.',
    rewritePrompt: '같은 언어입니다. 문장을 개선할까요?', rewrite: '문장 개선', changeTarget: '대상 언어 변경',
    error: '번역을 완료하지 못했습니다.', timedOut: '번역 시간이 초과되었습니다.', cancelled: '번역이 취소되었습니다.', signedOutError: 'ChatGPT 계정을 연결해 주세요.', unavailable: '번역 서비스를 시작할 수 없습니다.',
  },
  en: {
    workspace: 'Text translation', text: 'Text', image: 'Image', document: 'Document', capture: 'Screen capture', history: 'History',
    sourceLanguage: 'Source language', targetLanguage: 'Target language', auto: 'Detect language', swap: 'Swap languages', source: 'Source text', result: 'Translation',
    translate: 'Translate', cancel: 'Cancel', retry: 'Retry', copyResult: 'Copy translation', saveResult: 'Save translation', clear: 'Clear all',
    signedIn: 'ChatGPT account connected', signedOut: 'Connect your ChatGPT account to translate.', checking: 'Checking your account and settings.',
    shortcut: 'Shortcut: Not set', ready: 'Ready to translate.', running: 'Translating.', completed: 'Translation complete.', copied: 'Translation copied.', saved: 'Translation file saved.',
    empty: 'Enter source text to translate.', tooLarge: 'Source text must be at most 200,000 characters and 1,000,000 bytes.', loadError: 'Could not load translation settings.',
    rewritePrompt: 'The languages match. Improve the writing instead?', rewrite: 'Improve writing', changeTarget: 'Change target language',
    error: 'Could not complete the translation.', timedOut: 'The translation timed out.', cancelled: 'Translation cancelled.', signedOutError: 'Connect your ChatGPT account.', unavailable: 'The translation service is unavailable.',
  },
} as const;

function errorMessage(code: string, locale: AppLocale) {
  const labels = copy[locale];
  if (code.includes('timed_out')) return labels.timedOut;
  if (code.includes('cancelled')) return labels.cancelled;
  if (code.includes('signed_out')) return labels.signedOutError;
  if (code.includes('unavailable')) return labels.unavailable;
  return labels.error;
}

function sourceWithinBounds(text: string) {
  return Array.from(text).length <= MAX_SOURCE_CHARS
    && new TextEncoder().encode(text).length <= MAX_SOURCE_BYTES;
}

export function TextWorkspace({
  locale: localeOverride,
  onLocaleLoaded,
}: {
  locale?: AppLocale;
  onLocaleLoaded?: (locale: AppLocale) => void;
}) {
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [source, setSource] = useState('');
  const [profile, setProfile] = useState<TranslationProfile | null>(null);
  const [accountPhase, setAccountPhase] = useState<'checking' | 'signedIn' | 'signedOut'>('checking');
  const [notice, setNotice] = useState('');
  const [validationError, setValidationError] = useState('');
  const [loadError, setLoadError] = useState(false);
  const accountGeneration = useRef(0);
  const mounted = useRef(false);
  const { state, detectedLanguage, start, cancel, reset } = useTranslationJob();
  const locale = localeOverride ?? settings?.locale ?? 'ko';
  const labels = copy[locale];

  const refreshAccount = useCallback(async () => {
    const requestGeneration = ++accountGeneration.current;
    try {
      const snapshot = await getAccount();
      if (!mounted.current || requestGeneration !== accountGeneration.current) return;
      setAccountPhase(snapshot.account.state === 'signedIn' ? 'signedIn' : 'signedOut');
    } catch {
      if (mounted.current && requestGeneration === accountGeneration.current) setAccountPhase('signedOut');
    }
  }, []);

  useEffect(() => {
    mounted.current = true;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void invoke<AppSettings>('get_settings').then((loaded) => {
      if (disposed) return;
      const savedProfile = resolveDefaultProfile(loaded);
      if (!savedProfile) {
        setLoadError(true);
        return;
      }
      setSettings(loaded);
      setProfile({ ...savedProfile.profile, protectedTerms: [...savedProfile.profile.protectedTerms] });
      onLocaleLoaded?.(loaded.locale);
    }).catch(() => !disposed && setLoadError(true));
    try {
      void onAccountStateChanged(() => void refreshAccount()).then((stop) => {
        if (disposed) stop();
        else {
          unlisten = stop;
          void refreshAccount();
        }
      }).catch(() => !disposed && void refreshAccount());
    } catch {
      void refreshAccount();
    }
    return () => {
      disposed = true;
      mounted.current = false;
      accountGeneration.current += 1;
      unlisten?.();
    };
  }, [onLocaleLoaded, refreshAccount]);

  const effectiveProfile = useMemo(() => {
    if (!profile) return null;
    const glossaryTerms = profile.sourceLanguage === null ? [] : (settings?.glossary ?? [])
      .filter((entry) => entry.protectOnly
        && entry.sourceLanguage.toLowerCase() === profile.sourceLanguage?.toLowerCase()
        && entry.targetLanguage.toLowerCase() === profile.targetLanguage.toLowerCase())
      .map((entry) => entry.sourceTerm);
    return { ...profile, protectedTerms: [...new Set([...profile.protectedTerms, ...glossaryTerms])] };
  }, [profile, settings]);

  const sameLanguage = Boolean(effectiveProfile && (
    effectiveProfile.sourceLanguage?.toLowerCase() === effectiveProfile.targetLanguage.toLowerCase()
    || (effectiveProfile.sourceLanguage === null && detectedLanguage?.toLowerCase() === effectiveProfile.targetLanguage.toLowerCase())
  ));

  const run = (mode: TranslationMode = 'translate') => {
    setNotice('');
    if (!source.trim()) {
      setValidationError(labels.empty);
      return;
    }
    if (!sourceWithinBounds(source)) {
      setValidationError(labels.tooLarge);
      return;
    }
    if (!effectiveProfile || accountPhase !== 'signedIn') return;
    if (mode === 'translate' && sameLanguage) return;
    setValidationError('');
    void start({ text: source, profile: effectiveProfile, mode, secret: false });
  };

  const swapLanguages = () => {
    if (!profile) return;
    setProfile({
      ...profile,
      sourceLanguage: profile.targetLanguage,
      targetLanguage: profile.sourceLanguage ?? (profile.targetLanguage === 'ko' ? 'en' : 'ko'),
    });
  };

  const clearAll = () => {
    if (state.status === 'running') return;
    setSource('');
    setValidationError('');
    setNotice('');
    reset();
  };

  const copyResult = async () => {
    if (!state.text) return;
    try {
      await navigator.clipboard.writeText(state.text);
      setNotice(labels.copied);
    } catch {
      setValidationError(labels.error);
    }
  };

  const saveResult = () => {
    if (!state.text) return;
    const url = URL.createObjectURL(new Blob([state.text], { type: 'text/plain;charset=utf-8' }));
    const link = document.createElement('a');
    link.href = url;
    link.download = `smartcat-translation-${effectiveProfile?.targetLanguage ?? 'translated'}.txt`;
    link.click();
    URL.revokeObjectURL(url);
    setNotice(labels.saved);
  };

  const retry = () => run(sameLanguage ? 'rewrite' : 'translate');
  const jobError = state.status === 'failed' ? errorMessage(state.message, locale) : '';
  const status = state.status === 'running' ? labels.running
    : state.status === 'completed' ? labels.completed
      : notice || (accountPhase === 'signedIn' ? labels.ready : '');
  const disabled = loadError || !effectiveProfile || accountPhase !== 'signedIn';

  return (
    <section className="text-workspace" aria-label={labels.workspace}>
      <nav className="workspace-tabs" role="tablist" aria-label={labels.workspace}>
        <button type="button" role="tab" aria-selected="true">{labels.text}</button>
        {[labels.image, labels.document, labels.capture, labels.history].map((label) => (
          <button key={label} type="button" role="tab" aria-selected="false" disabled>{label}</button>
        ))}
      </nav>

      <div className="workspace-meta">
        <span className={accountPhase === 'signedIn' ? 'connected' : ''}>
          {accountPhase === 'checking' ? labels.checking : accountPhase === 'signedIn' ? labels.signedIn : labels.signedOut}
        </span>
        <span>{labels.shortcut}</span>
      </div>

      {loadError && <p role="alert">{labels.loadError}</p>}
      <div className="language-bar">
        <label>{labels.sourceLanguage}
          <select
            value={profile?.sourceLanguage ?? 'auto'}
            disabled={!profile || state.status === 'running'}
            onChange={(event) => setProfile((current) => current && ({ ...current, sourceLanguage: event.target.value === 'auto' ? null : event.target.value }))}
          >
            <option value="auto">{labels.auto}</option>
            {SUPPORTED_LANGUAGES.map((language) => <option key={language.code} value={language.code}>{languageLabel(language, locale)}</option>)}
          </select>
        </label>
        <button type="button" className="swap-button" aria-label={labels.swap} onClick={swapLanguages} disabled={!profile || state.status === 'running'}>⇄</button>
        <label>{labels.targetLanguage}
          <select
            value={profile?.targetLanguage ?? 'ko'}
            disabled={!profile || state.status === 'running'}
            onChange={(event) => setProfile((current) => current && ({ ...current, targetLanguage: event.target.value }))}
          >
            {SUPPORTED_LANGUAGES.map((language) => <option key={language.code} value={language.code}>{languageLabel(language, locale)}</option>)}
          </select>
        </label>
      </div>

      {sameLanguage && <aside className="rewrite-suggestion" aria-live="polite">
        <span>{labels.rewritePrompt}</span>
        <button type="button" onClick={() => run('rewrite')} disabled={disabled || state.status === 'running'}>{labels.rewrite}</button>
        <button type="button" onClick={() => setProfile((current) => current && ({ ...current, targetLanguage: current.targetLanguage === 'ko' ? 'en' : 'ko' }))}>{labels.changeTarget}</button>
      </aside>}

      <div className="translation-grid">
        <div className="translation-pane source-pane">
          <label htmlFor="translation-source">{labels.source}</label>
          <textarea
            id="translation-source"
            aria-label={labels.source}
            value={source}
            onChange={(event) => { setSource(event.target.value); setValidationError(''); }}
            onKeyDown={(event) => {
              if ((event.ctrlKey || event.metaKey) && event.key === 'Enter') { event.preventDefault(); run(); }
              if (event.key === 'Escape' && state.status === 'running') { event.preventDefault(); void cancel(); }
            }}
          />
          <small aria-live="off">{Array.from(source).length.toLocaleString(locale === 'ko' ? 'ko-KR' : 'en-US')} / {MAX_SOURCE_CHARS.toLocaleString()}</small>
        </div>
        <div className="translation-pane result-pane">
          <label htmlFor="translation-result">{labels.result}</label>
          <textarea id="translation-result" aria-label={labels.result} value={state.text} readOnly />
        </div>
      </div>

      <div className="workspace-actions">
        <button
          type="button"
          className="primary-action"
          disabled={disabled || (sameLanguage && state.status !== 'running')}
          onClick={state.status === 'running' ? () => void cancel() : () => run()}
        >
          {state.status === 'running' ? labels.cancel : labels.translate}
        </button>
        <button type="button" onClick={() => void copyResult()} disabled={!state.text}>{labels.copyResult}</button>
        <button type="button" onClick={saveResult} disabled={!state.text}>{labels.saveResult}</button>
        <button type="button" onClick={clearAll} disabled={state.status === 'running' || (!source && !state.text)}>{labels.clear}</button>
      </div>

      <div className="workspace-status" aria-live="polite" role="status">{status}</div>
      {(validationError || jobError) && <p role="alert" aria-live="polite">{validationError || jobError}</p>}
      {state.status === 'failed' && <button type="button" onClick={retry} disabled={disabled}>{labels.retry}</button>}
    </section>
  );
}
