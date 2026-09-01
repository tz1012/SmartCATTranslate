import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { TranslationRequest } from '../../lib/types';
import { SecretModeSwitch,useSecretMode } from '../history/secretMode';
import { saveHistoryRecord } from '../history/historyApi';
import { useTranslationJob } from './useTranslationJob';

type Locale = 'ko' | 'en';
type PopupPayload = {
  requestId: string;
  request: TranslationRequest | null;
  profileName: string;
  locale: Locale;
  error: string | null;
};

const labels = {
  ko: { source: '원문', result: '번역문', translating: '번역 중…', cancel: '취소', retry: '다시 시도', copy: '복사', copied: '복사됨', listen: '듣기', stop: '중지', pin: '창 고정', unpin: '고정 해제', open: '전체 창에서 열기', close: '닫기', waiting: '빠른 번역 대기 중', failed: '번역을 완료하지 못했습니다.', signedOut: 'ChatGPT 계정을 연결해 주세요.', rate: '사용 한도에 도달했습니다. 잠시 후 다시 시도해 주세요.', offline: '인터넷 연결과 ChatGPT 로그인을 확인해 주세요.' },
  en: { source: 'Source', result: 'Translation', translating: 'Translating…', cancel: 'Cancel', retry: 'Retry', copy: 'Copy', copied: 'Copied', listen: 'Listen', stop: 'Stop', pin: 'Pin window', unpin: 'Unpin', open: 'Open in main window', close: 'Close', waiting: 'Waiting for quick translation', failed: 'Could not complete the translation.', signedOut: 'Connect your ChatGPT account.', rate: 'Usage limit reached. Try again later.', offline: 'Check your internet connection and ChatGPT sign-in.' },
} as const;

function friendlyError(code: string, locale: Locale) {
  const text = labels[locale];
  if (code.includes('signed_out') || code.includes('account')) return text.signedOut;
  if (code.includes('rate') || code.includes('limit')) return text.rate;
  if (code.includes('unavailable') || code.includes('transport')) return text.offline;
  if (code === 'no_selection' || code === 'clipboard_unchanged') return locale === 'ko' ? '번역할 텍스트를 선택해 주세요.' : 'Select text to translate.';
  if (code === 'clipboard_restore_failed') return locale === 'ko' ? '클립보드를 안전하게 복원하지 못해 번역을 중단했습니다.' : 'Translation stopped because the clipboard could not be safely restored.';
  return text.failed;
}

export function QuickPopup() {
  const [payload, setPayload] = useState<PopupPayload | null>(null);
  const [pinned, setPinned] = useState(false);

  useEffect(() => {
    let disposed = false;
    let stop: (() => void) | undefined;
    void listen<PopupPayload>('quick-popup-request', (event) => {
      if (!disposed) setPayload(event.payload);
    }).then((unlisten) => { if (disposed) unlisten(); else stop = unlisten; });
    return () => { disposed = true; stop?.(); };
  }, []);

  useEffect(() => {
    const close = () => void invoke('close_quick_popup');
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') { event.preventDefault(); close(); }
    };
    let blurTimer: number | undefined;
    const onBlur = () => {
      if (!pinned) blurTimer = window.setTimeout(() => {
        if (!document.hasFocus() && !document.activeElement?.closest('[data-popup-action]')) close();
      }, 150);
    };
    const onFocus = () => { if (blurTimer) window.clearTimeout(blurTimer); };
    window.addEventListener('keydown', onKey);
    window.addEventListener('blur', onBlur);
    window.addEventListener('focus', onFocus);
    return () => {
      if (blurTimer) window.clearTimeout(blurTimer);
      window.removeEventListener('keydown', onKey);
      window.removeEventListener('blur', onBlur);
      window.removeEventListener('focus', onFocus);
    };
  }, [pinned]);

  return <main className="quick-popup-shell" data-theme="system">
    {payload ? <PopupRequest key={payload.requestId} payload={payload} pinned={pinned} onPin={() => setPinned((value) => !value)} /> : <p className="quick-popup-waiting">빠른 번역 대기 중</p>}
  </main>;
}

function PopupRequest({ payload, pinned, onPin }: { payload: PopupPayload; pinned: boolean; onPin: () => void }) {
  const { state, listenerState, start, cancel, reset, retryListener } = useTranslationJob();
  const [copied, setCopied] = useState(false);
  const [speaking, setSpeaking] = useState(false);
  const started = useRef(false);
  const text = labels[payload.locale];
  const [secret,setSecret]=useSecretMode();
  const savedHistoryJob=useRef<string|undefined>(undefined);
  const activeSecret=useRef(false);

  useEffect(() => {
    if (!payload.request || payload.error || listenerState !== 'ready' || started.current) return;
    started.current = true;
    activeSecret.current=secret;
    void start({...payload.request,secret});
  }, [listenerState, payload, secret,start]);

  const retry = () => {
    if (!payload.request || state.status === 'running') return;
    if (errorCode === 'translation_listener_unavailable') {
      retryListener();
      return;
    }
    reset();
    started.current = false;
    queueMicrotask(() => { started.current = true;activeSecret.current=secret; void start({...payload.request!,secret}); });
  };
  const errorCode = payload.error ?? (state.status === 'failed' ? state.message : '');
  const sourceLanguage = payload.request?.profile.sourceLanguage?.toUpperCase() ?? (payload.locale === 'ko' ? '자동 감지' : 'Auto');
  const targetLanguage = payload.request?.profile.targetLanguage.toUpperCase() ?? '—';
  const voice = typeof speechSynthesis === 'undefined' ? undefined : speechSynthesis.getVoices().find((candidate) => candidate.lang.toLowerCase().startsWith(targetLanguage.toLowerCase()));
  const toggleSpeech = () => {
    if (!voice || !state.text) return;
    if (speaking) { speechSynthesis.cancel(); setSpeaking(false); return; }
    const utterance = new SpeechSynthesisUtterance(state.text);
    utterance.lang = voice.lang;
    utterance.voice = voice;
    utterance.onend = utterance.onerror = () => setSpeaking(false);
    setSpeaking(true);
    speechSynthesis.speak(utterance);
  };

  useEffect(() => () => { if (typeof speechSynthesis !== 'undefined') speechSynthesis.cancel(); }, []);
  useEffect(()=>{if(state.status!=='completed'||savedHistoryJob.current===state.jobId||!payload.request)return;savedHistoryJob.current=state.jobId;void saveHistoryRecord({kind:'popup',sourceLanguage:payload.request.profile.sourceLanguage,targetLanguage:payload.request.profile.targetLanguage,source:payload.request.text,result:state.text,displayName:null,warningCount:0,secret:activeSecret.current}).catch(()=>undefined);},[payload.request,state]);

  return <section className="quick-popup" aria-label="SmartCAT quick translation">
    <header className="quick-popup-header">
      <div><strong>SmartCAT</strong><span>{sourceLanguage} → {targetLanguage} · {payload.profileName}</span></div>
      <div className="quick-popup-header-actions">
        <button data-popup-action type="button" aria-pressed={pinned} aria-label={pinned ? text.unpin : text.pin} title={pinned ? text.unpin : text.pin} onClick={onPin}>{pinned ? '●' : '○'}</button>
        <button data-popup-action type="button" aria-label={text.close} title={text.close} onClick={() => void invoke('close_quick_popup')}>×</button>
      </div>
    </header>
    <SecretModeSwitch locale={payload.locale} value={secret} onChange={setSecret}/>
    <div className="quick-popup-content">
      {payload.request && <article><h2>{text.source}</h2><p>{payload.request.text}</p></article>}
      <article aria-live="polite" aria-busy={state.status === 'running'}>
        <h2>{text.result}</h2>
        {errorCode ? <p className="quick-popup-error" role="alert">{friendlyError(errorCode, payload.locale)}</p>
          : <p className={state.text ? '' : 'quick-popup-placeholder'}>{state.text || text.translating}</p>}
      </article>
    </div>
    <footer className="quick-popup-actions">
      {state.status === 'running' && <button data-popup-action type="button" onClick={() => void cancel()}>{text.cancel}</button>}
      {errorCode && payload.request && <button data-popup-action type="button" onClick={retry}>{text.retry}</button>}
      <button data-popup-action type="button" disabled={!state.text} onClick={async () => { try { await navigator.clipboard.writeText(state.text); setCopied(true); } catch { /* platform denied copy */ } }}>{copied ? text.copied : text.copy}</button>
      <button data-popup-action type="button" disabled={!state.text || !voice} onClick={toggleSpeech}>{speaking ? text.stop : text.listen}</button>
      <button data-popup-action type="button" onClick={() => void invoke('open_main_window')}>{text.open}</button>
    </footer>
  </section>;
}
