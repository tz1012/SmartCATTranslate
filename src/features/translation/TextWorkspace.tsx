import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { CompletedTextTranslation, Field, GlossaryMapping, TranslationMode, TranslationProfile } from '../../lib/types';
import { getAccount, onAccountStateChanged } from '../account/accountApi';
import type { AppLocale, AppSettings, Theme } from '../settings/SettingsView';
import { resolveDefaultProfile } from '../settings/defaultProfile';
import { languageLabel, SUPPORTED_LANGUAGES } from '../settings/languages';
import { useTranslationJob } from './useTranslationJob';
import { saveTranslationText } from './translationApi';
import { saveHistoryRecord } from '../history/historyApi';
import { SecretModeSwitch, useSecretMode } from '../history/secretMode';

const MAX_SOURCE_CHARS = 200_000;
const MAX_SOURCE_BYTES = 1_000_000;
const AUTO_TRANSLATE_DEBOUNCE_MS = 500;
const TEXT_TRANSLATION_SUPPORT_CODES = new Set([
  'invalid_translation_input',
  'unsafe_translation_workspace',
  'invalid_translation_output',
  'translation_size_limit',
  'translation_tool_rejected',
  'translation_runtime_unavailable',
  'translation_protocol_violation',
  'translation_timed_out',
  'translation_cancelled',
  'translation_shutting_down',
  'translation_service_unavailable',
  'translation_listener_unavailable',
  'translation_cancel_failed',
  'translation_start_failed',
  'rewrite_suggested',
]);

const copy = {
  ko: {
    workspace: '텍스트 번역', text: '텍스트', image: '이미지', document: '문서', capture: '화면 캡처', history: '기록',
    sourceLanguage: '원문 언어', targetLanguage: '대상 언어', auto: '언어 감지', swap: '언어 바꾸기', source: '원문', result: '번역문',
    translate: '번역', cancel: '취소', retry: '다시 시도', copyResult: '번역문 복사', saveResult: '번역문 저장', clear: '모두 지우기',
    signedIn: 'ChatGPT 계정 연결됨', signedOut: '번역하려면 ChatGPT 계정을 연결해 주세요.', checking: '계정과 설정을 확인하는 중입니다.',
    shortcut: '단축키: 설정되지 않음', ready: '번역할 준비가 되었습니다.', running: '번역 중입니다.', completed: '번역이 완료되었습니다.', copied: '번역문을 복사했습니다.', saved: '번역문 파일을 저장했습니다.',
    empty: '번역할 원문을 입력해 주세요.', tooLarge: '원문은 200,000자와 1,000,000바이트 이하여야 합니다.', loadError: '번역 설정을 불러올 수 없습니다.',
    rewritePrompt: '같은 언어입니다. 문장을 개선할까요?', rewrite: '문장 개선', changeTarget: '대상 언어 변경',
    error: '번역을 완료하지 못했습니다.', timedOut: '번역 시간이 초과되었습니다.', cancelled: '번역이 취소되었습니다.', signedOutError: 'ChatGPT 계정을 연결해 주세요.', unavailable: '번역 서비스를 시작할 수 없습니다.', cancelFailed: '번역 취소를 완료하지 못했습니다.', copyFailed: '번역문을 복사하지 못했습니다.', saveFailed: '번역문 파일을 저장하지 못했습니다.', supportCode: '지원 코드:',
  },
  en: {
    workspace: 'Text translation', text: 'Text', image: 'Image', document: 'Document', capture: 'Screen capture', history: 'History',
    sourceLanguage: 'Source language', targetLanguage: 'Target language', auto: 'Detect language', swap: 'Swap languages', source: 'Source text', result: 'Translation',
    translate: 'Translate', cancel: 'Cancel', retry: 'Retry', copyResult: 'Copy translation', saveResult: 'Save translation', clear: 'Clear all',
    signedIn: 'ChatGPT account connected', signedOut: 'Connect your ChatGPT account to translate.', checking: 'Checking your account and settings.',
    shortcut: 'Shortcut: Not set', ready: 'Ready to translate.', running: 'Translating.', completed: 'Translation complete.', copied: 'Translation copied.', saved: 'Translation file saved.',
    empty: 'Enter source text to translate.', tooLarge: 'Source text must be at most 200,000 characters and 1,000,000 bytes.', loadError: 'Could not load translation settings.',
    rewritePrompt: 'The languages match. Improve the writing instead?', rewrite: 'Improve writing', changeTarget: 'Change target language',
    error: 'Could not complete the translation.', timedOut: 'The translation timed out.', cancelled: 'Translation cancelled.', signedOutError: 'Connect your ChatGPT account.', unavailable: 'The translation service is unavailable.', cancelFailed: 'Could not cancel the translation.', copyFailed: 'Could not copy the translation.', saveFailed: 'Could not save the translation file.', supportCode: 'Support code:',
  },
} as const;

function errorMessage(code: string, locale: AppLocale) {
  const labels = copy[locale];
  const safeCode = TEXT_TRANSLATION_SUPPORT_CODES.has(code) ? code : 'translation_start_failed';
  const message = safeCode.includes('timed_out') ? labels.timedOut
    : safeCode.includes('cancelled') ? labels.cancelled
      : safeCode.includes('cancel_failed') ? labels.cancelFailed
        : safeCode.includes('listener_unavailable') ? labels.unavailable
          : safeCode.includes('signed_out') ? labels.signedOutError
            : safeCode.includes('unavailable') ? labels.unavailable : labels.error;
  return `${message} ${labels.supportCode} ${safeCode}`;
}

function sourceWithinBounds(text: string) {
  return Array.from(text).length <= MAX_SOURCE_CHARS
    && new TextEncoder().encode(text).length <= MAX_SOURCE_BYTES;
}

export function TextWorkspace({
  locale: localeOverride,
  onActivityChange,
  importedTranslation,
  onImportedTranslationConsumed,
  onPreferencesLoaded,
}: {
  locale?: AppLocale;
  onActivityChange?: (active: boolean) => void;
  importedTranslation?: CompletedTextTranslation;
  onImportedTranslationConsumed?: () => void;
  onPreferencesLoaded?: (locale: AppLocale, theme: Theme) => void;
}) {
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [source, setSource] = useState(() => importedTranslation?.source ?? '');
  const [importedResult, setImportedResult] = useState<string | null>(() => importedTranslation?.translation ?? null);
  const [profile, setProfile] = useState<TranslationProfile | null>(null);
  const [field, setField] = useState<Field>('general');
  const [accountPhase, setAccountPhase] = useState<'checking' | 'signedIn' | 'signedOut'>('checking');
  const [notice, setNotice] = useState('');
  const [validationError, setValidationError] = useState('');
  const [loadError, setLoadError] = useState(false);
  const [editRevision, setEditRevision] = useState(0);
  const [composing, setComposing] = useState(false);
  const [secret,setSecret]=useSecretMode();
  const savedHistoryJob=useRef<string|undefined>(undefined);
  const activeSecret=useRef(false);
  const accountGeneration = useRef(0);
  const autoStartTimer = useRef<number | undefined>(undefined);
  const startedRevision = useRef(0);
  const mounted = useRef(false);
  const { state, detectedLanguage, listenerState, start, cancel, reset, retryListener } = useTranslationJob();
  const locale = localeOverride ?? settings?.locale ?? 'ko';
  const labels = copy[locale];
  const displayedText = importedResult ?? state.text;

  useEffect(() => {
    if (!importedTranslation) return;
    onImportedTranslationConsumed?.();
  }, [importedTranslation, onImportedTranslationConsumed]);

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
      setField(savedProfile.field);
      onPreferencesLoaded?.(loaded.locale, loaded.theme);
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
  }, [onPreferencesLoaded, refreshAccount]);

  const effectiveProfile = useMemo(() => {
    if (!profile) return null;
    const glossaryTerms = (settings?.glossary ?? [])
      .filter((entry) => entry.protectOnly
        && (profile.sourceLanguage === null || entry.sourceLanguage.toLowerCase() === profile.sourceLanguage.toLowerCase())
        && entry.targetLanguage.toLowerCase() === profile.targetLanguage.toLowerCase())
      .map((entry) => entry.sourceTerm);
    return { ...profile, protectedTerms: [...new Set([...profile.protectedTerms, ...glossaryTerms])] };
  }, [profile, settings]);

  const glossary = useMemo<GlossaryMapping[]>(() => {
    if (!profile) return [];
    return (settings?.glossary ?? [])
      .filter((entry) => !entry.protectOnly
        && (profile.sourceLanguage === null || entry.sourceLanguage.toLowerCase() === profile.sourceLanguage.toLowerCase())
        && entry.targetLanguage.toLowerCase() === profile.targetLanguage.toLowerCase())
      .map((entry) => ({ sourceTerm: entry.sourceTerm, targetTerm: entry.targetTerm }));
  }, [profile, settings]);

  const sameLanguage = Boolean(effectiveProfile && (
    effectiveProfile.sourceLanguage?.toLowerCase() === effectiveProfile.targetLanguage.toLowerCase()
    || (effectiveProfile.sourceLanguage === null && detectedLanguage?.toLowerCase() === effectiveProfile.targetLanguage.toLowerCase())
  ));

  const runText = (text: string, mode: TranslationMode = 'translate', useDetectedLanguage = true) => {
    setNotice('');
    if (!text.trim()) {
      setValidationError(labels.empty);
      return;
    }
    if (!sourceWithinBounds(text)) {
      setValidationError(labels.tooLarge);
      return;
    }
    if (!effectiveProfile || accountPhase !== 'signedIn' || listenerState !== 'ready') return;
    const sourceMatchesTarget = effectiveProfile.sourceLanguage?.toLowerCase() === effectiveProfile.targetLanguage.toLowerCase()
      || (useDetectedLanguage && effectiveProfile.sourceLanguage === null && detectedLanguage?.toLowerCase() === effectiveProfile.targetLanguage.toLowerCase());
    if (mode === 'translate' && sourceMatchesTarget) return;
    setValidationError('');
    activeSecret.current=secret;
    void start({ text, profile: effectiveProfile, field, glossary, mode, secret });
  };

  const run = (mode: TranslationMode = 'translate') => {
    if (autoStartTimer.current !== undefined) {
      window.clearTimeout(autoStartTimer.current);
      autoStartTimer.current = undefined;
    }
    setImportedResult(null);
    startedRevision.current = editRevision;
    runText(source, mode);
  };

  const clearBoundResult = () => {
    setImportedResult(null);
    if (state.status === 'completed' || (state.status === 'failed' && !state.pendingCleanup)) reset();
    setNotice('');
    setValidationError('');
  };

  const swapLanguages = () => {
    if (!profile || state.status === 'running' || (state.status === 'failed' && state.pendingCleanup)) return;
    clearBoundResult();
    setProfile({
      ...profile,
      sourceLanguage: profile.targetLanguage,
      targetLanguage: profile.sourceLanguage ?? (profile.targetLanguage === 'ko' ? 'en' : 'ko'),
    });
  };

  const clearAll = () => {
    if (state.status === 'running' || pendingCleanup) return;
    setSource('');
    setImportedResult(null);
    setValidationError('');
    setNotice('');
    reset();
  };

  const copyResult = async () => {
    if (!displayedText) return;
    try {
      await navigator.clipboard.writeText(displayedText);
      setNotice(labels.copied);
    } catch {
      setValidationError(labels.copyFailed);
    }
  };

  const saveResult = async () => {
    if (!displayedText) return;
    setValidationError('');
    try {
      const result = await saveTranslationText(displayedText, effectiveProfile?.targetLanguage ?? 'translated', locale);
      if (result.status === 'saved') setNotice(labels.saved);
    } catch {
      setValidationError(labels.saveFailed);
    }
  };

  const listenerFailed = state.status === 'failed' && state.message === 'translation_listener_unavailable';
  const retry = () => listenerFailed
    ? retryListener()
    : state.status === 'failed' && state.pendingCleanup
      ? void cancel()
      : run(sameLanguage ? 'rewrite' : 'translate');
  const jobError = state.status === 'failed' ? errorMessage(state.message, locale) : '';
  const status = notice || (state.status === 'running' ? labels.running
    : importedResult !== null || state.status === 'completed' ? labels.completed
      : accountPhase === 'signedIn' ? labels.ready
        : accountPhase === 'signedOut' ? labels.signedOut : labels.checking);
  const pendingCleanup = state.status === 'failed' && Boolean(state.pendingCleanup);
  const activityActive = state.status === 'running' || pendingCleanup;
  const disabled = loadError || !effectiveProfile || accountPhase !== 'signedIn' || listenerState !== 'ready' || pendingCleanup;

  useEffect(() => {
    if (autoStartTimer.current !== undefined) {
      window.clearTimeout(autoStartTimer.current);
      autoStartTimer.current = undefined;
    }
    if (editRevision === 0 || editRevision <= startedRevision.current || composing || activityActive
      || disabled || sameLanguage || !source.trim() || !sourceWithinBounds(source) || !effectiveProfile) return;

    autoStartTimer.current = window.setTimeout(() => {
      autoStartTimer.current = undefined;
      startedRevision.current = editRevision;
      setNotice('');
      setValidationError('');
      activeSecret.current = secret;
      void start({ text: source, profile: effectiveProfile, field, glossary, mode: 'translate', secret });
    }, AUTO_TRANSLATE_DEBOUNCE_MS);
    return () => {
      if (autoStartTimer.current !== undefined) {
        window.clearTimeout(autoStartTimer.current);
        autoStartTimer.current = undefined;
      }
    };
  }, [activityActive, composing, disabled, editRevision, effectiveProfile, field, glossary, sameLanguage, secret, source, start]);

  useEffect(() => {
    onActivityChange?.(activityActive);
  }, [activityActive, onActivityChange]);

  useEffect(() => () => onActivityChange?.(false), [onActivityChange]);

  useEffect(()=>{if(state.status!=='completed'||savedHistoryJob.current===state.jobId||!effectiveProfile)return;savedHistoryJob.current=state.jobId;void saveHistoryRecord({kind:'text',sourceLanguage:effectiveProfile.sourceLanguage,targetLanguage:effectiveProfile.targetLanguage,source,result:state.text,displayName:null,warningCount:0,secret:activeSecret.current}).catch(()=>undefined);},[effectiveProfile,source,state]);

  return (
    <section className="text-workspace" aria-label={labels.workspace} onKeyDownCapture={(event) => {
      if (event.key === 'Escape' && activityActive) {
        event.preventDefault();
        void cancel();
      }
    }}>
      {loadError && <p role="alert">{labels.loadError}</p>}
      <SecretModeSwitch locale={locale} value={secret} onChange={setSecret}/>
      <div className="language-bar">
        <label>{labels.sourceLanguage}
          <select
            value={profile?.sourceLanguage ?? 'auto'}
            disabled={!profile || state.status === 'running' || pendingCleanup}
            onChange={(event) => {
              clearBoundResult();
              setProfile((current) => current && ({ ...current, sourceLanguage: event.target.value === 'auto' ? null : event.target.value }));
            }}
          >
            <option value="auto">{labels.auto}</option>
            {SUPPORTED_LANGUAGES.map((language) => <option key={language.code} value={language.code}>{languageLabel(language, locale)}</option>)}
          </select>
        </label>
        <button type="button" className="swap-button" aria-label={labels.swap} onClick={swapLanguages} disabled={!profile || state.status === 'running' || pendingCleanup}>⇄</button>
        <label>{labels.targetLanguage}
          <select
            value={profile?.targetLanguage ?? 'ko'}
            disabled={!profile || state.status === 'running' || pendingCleanup}
            onChange={(event) => {
              clearBoundResult();
              setProfile((current) => current && ({ ...current, targetLanguage: event.target.value }));
            }}
          >
            {SUPPORTED_LANGUAGES.map((language) => <option key={language.code} value={language.code}>{languageLabel(language, locale)}</option>)}
          </select>
        </label>
      </div>

      {sameLanguage && <aside className="rewrite-suggestion" aria-live="polite">
        <span>{labels.rewritePrompt}</span>
        <button type="button" onClick={() => run('rewrite')} disabled={disabled || state.status === 'running'}>{labels.rewrite}</button>
        <button type="button" disabled={state.status === 'running' || pendingCleanup} onClick={() => {
          clearBoundResult();
          setProfile((current) => current && ({ ...current, targetLanguage: current.targetLanguage === 'ko' ? 'en' : 'ko' }));
        }}>{labels.changeTarget}</button>
      </aside>}

      <div className="translation-grid">
        <div className="translation-pane source-pane">
          <label htmlFor="translation-source">{labels.source}</label>
          <textarea
            id="translation-source"
            aria-label={labels.source}
            value={source}
            readOnly={activityActive}
            onCompositionStart={() => setComposing(true)}
            onCompositionEnd={() => setComposing(false)}
            onChange={(event) => {
              if (activityActive) return;
              clearBoundResult();
              setSource(event.target.value);
              setEditRevision((revision) => revision + 1);
            }}
            onPaste={(event) => {
              if (activityActive) return;
              event.preventDefault();
              const pasted = event.clipboardData.getData('text');
              const start = event.currentTarget.selectionStart;
              const end = event.currentTarget.selectionEnd;
              const nextSource = source.slice(0, start) + pasted + source.slice(end);
              clearBoundResult();
              setSource(nextSource);
              runText(nextSource, 'translate', false);
            }}
            onKeyDown={(event) => {
              if ((event.ctrlKey || event.metaKey) && event.key === 'Enter') { event.preventDefault(); run(); }
            }}
          />
          <small aria-live="off">{Array.from(source).length.toLocaleString(locale === 'ko' ? 'ko-KR' : 'en-US')} / {MAX_SOURCE_CHARS.toLocaleString()}</small>
        </div>
        <div className="translation-pane result-pane">
          <label htmlFor="translation-result">{labels.result}</label>
          <textarea id="translation-result" aria-label={labels.result} value={displayedText} readOnly />
        </div>
      </div>

      <footer className="workspace-footer">
        <div className="workspace-actions">
          <button
            type="button"
            className="primary-action"
            disabled={state.status === 'running' ? false : disabled || sameLanguage}
            onClick={state.status === 'running' ? () => void cancel() : () => run()}
          >
            {state.status === 'running' ? labels.cancel : labels.translate}
          </button>
          <button type="button" onClick={() => void copyResult()} disabled={!displayedText}>{labels.copyResult}</button>
          <button type="button" onClick={() => void saveResult()} disabled={!displayedText}>{labels.saveResult}</button>
          <button type="button" onClick={clearAll} disabled={state.status === 'running' || pendingCleanup || (!source && !displayedText)}>{labels.clear}</button>
        </div>

        <div className="workspace-status" aria-live="polite" role="status">{status}</div>
        {(validationError || jobError) && <p role="alert" aria-live="polite">{validationError || jobError}</p>}
        {state.status === 'failed' && <button type="button" onClick={retry} disabled={listenerFailed ? listenerState !== 'failed' : state.pendingCleanup ? false : disabled}>{labels.retry}</button>}
      </footer>
    </section>
  );
}
