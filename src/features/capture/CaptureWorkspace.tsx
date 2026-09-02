import { useCallback, useEffect, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { cancelImageTranslation, chooseImage, startScreenCapture, translateImage } from './captureApi';
import type { CaptureJobResult, CaptureProgress } from './types';
import { CaptureResult } from './CaptureResult';
import { SecretModeSwitch,useSecretMode } from '../history/secretMode';

type CaptureErrorStage = 'capture' | 'translation' | 'import';

type CaptureError = {
  stage: CaptureErrorStage;
  supportCode: string;
};

type CaptureSessionEnded = {
  sessionId: string;
  status: 'failed' | 'cancelled';
  reason?: string;
};

type CaptureSourceReady = CaptureJobResult & {
  captureSessionId?: string;
};

const MAX_PENDING_CAPTURE_EVENTS = 8;

function rememberPendingEvent<T>(events: Map<string, T>, sessionId: string, event: T) {
  events.delete(sessionId);
  events.set(sessionId, event);
  while (events.size > MAX_PENDING_CAPTURE_EVENTS) {
    const oldest = events.keys().next().value;
    if (oldest === undefined) break;
    events.delete(oldest);
  }
}

const supportCodePattern = /^(?:capture_|screen_capture_|translation_|ocr_|image_|settings_|foreground_|invalid_|unsupported_)[a-z_]{1,48}$/;

const fallbackSupportCodes: Record<CaptureErrorStage, string> = {
  capture: 'capture_failed',
  translation: 'translation_failed',
  import: 'image_import_failed',
};

function toCaptureError(stage: CaptureErrorStage, reason: unknown): CaptureError {
  const candidate = typeof reason === 'string' ? reason : reason instanceof Error ? reason.message : '';
  return {
    stage,
    supportCode: supportCodePattern.test(candidate) ? candidate : fallbackSupportCodes[stage],
  };
}

export function CaptureWorkspace({ locale }: { locale: 'ko' | 'en' }) {
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<CaptureJobResult>();
  const [error, setError] = useState<CaptureError>();
  const [progress, setProgress] = useState<CaptureProgress>();
  const [captureListenersReady, setCaptureListenersReady] = useState(false);
  const activeCaptureSession = useRef<string | undefined>(undefined);
  const captureStartPending = useRef(false);
  const pendingCaptureTerminalEvents = useRef(new Map<string, CaptureSessionEnded>());
  const pendingCaptureSourceEvents = useRef(new Map<string, CaptureSourceReady>());
  const ko = locale === 'ko';
  const [secret,setSecret]=useSecretMode();
  const finishCaptureSession = useCallback((status: CaptureSessionEnded['status'], reason?: unknown) => {
    activeCaptureSession.current = undefined;
    setBusy(false);
    setProgress(undefined);
    setError(status === 'failed' ? toCaptureError('capture', reason) : undefined);
  }, []);
  useEffect(() => {
    let disposed = false;
    let stops: Array<() => void> = [];
    const sourceReady = (payload: CaptureSourceReady) => {
      activeCaptureSession.current = undefined;
      setResult(payload);
      setError(undefined);
      setBusy(false);
    };
    void Promise.all([
      listen<CaptureSourceReady>('capture-source-ready', (event) => {
        const sessionId = event.payload.captureSessionId;
        if (!sessionId) {
          sourceReady(event.payload);
        } else if (activeCaptureSession.current === sessionId) {
          sourceReady(event.payload);
        } else if (captureStartPending.current) {
          rememberPendingEvent(pendingCaptureSourceEvents.current, sessionId, event.payload);
        }
      }),
      listen<CaptureSessionEnded>('capture-session-ended', (event) => {
        if (activeCaptureSession.current === event.payload.sessionId) {
          finishCaptureSession(event.payload.status, event.payload.reason);
        } else if (captureStartPending.current) {
          rememberPendingEvent(
            pendingCaptureTerminalEvents.current,
            event.payload.sessionId,
            event.payload,
          );
        }
      }),
    ]).then((unlisteners) => {
      if (disposed) {
        unlisteners.forEach((unlisten) => unlisten());
      } else {
        stops = unlisteners;
        setCaptureListenersReady(true);
      }
    });
    return () => {
      disposed = true;
      stops.forEach((stop) => stop());
    };
  }, [finishCaptureSession]);
  useEffect(() => { let disposed = false; let stop: (() => void) | undefined; void listen<CaptureProgress>('capture-progress', (event) => { if (!result || event.payload.jobId === result.jobId) setProgress(event.payload); }).then((unlisten) => { if (disposed) unlisten(); else stop = unlisten; }); return () => { disposed = true; stop?.(); }; }, [result]);
  const start = async () => {
    setBusy(true); setError(undefined);
    captureStartPending.current = true;
    pendingCaptureTerminalEvents.current.clear();
    pendingCaptureSourceEvents.current.clear();
    try {
      const { sessionId } = await startScreenCapture();
      captureStartPending.current = false;
      const sourceEvent = pendingCaptureSourceEvents.current.get(sessionId);
      const terminalEvent = pendingCaptureTerminalEvents.current.get(sessionId);
      pendingCaptureSourceEvents.current.clear();
      pendingCaptureTerminalEvents.current.clear();
      if (sourceEvent) {
        activeCaptureSession.current = undefined;
        setResult(sourceEvent);
        setError(undefined);
        setBusy(false);
      } else if (terminalEvent) {
        finishCaptureSession(terminalEvent.status, terminalEvent.reason);
      } else {
        activeCaptureSession.current = sessionId;
      }
    }
    catch (reason) {
      captureStartPending.current = false;
      pendingCaptureSourceEvents.current.clear();
      pendingCaptureTerminalEvents.current.clear();
      finishCaptureSession('failed', reason);
    }
  };
  const translate = async () => {
    if (!result) return; setBusy(true); setError(undefined); setProgress({ jobId: result.jobId, stage: 'ocr', percent: 5 });
    try { setResult(await translateImage(result.jobId, ['ko', 'en'],secret)); }
    catch (reason) { setError(toCaptureError('translation', reason)); }
    finally { setBusy(false); }
  };
  const choose = async () => {
    setBusy(true); setError(undefined);
    try { const imported = await chooseImage(); if (imported) setResult(imported); }
    catch (reason) { setError(toCaptureError('import', reason)); }
    finally { setBusy(false); }
  };
  return <section className="capture-workspace" aria-labelledby="capture-title">
    <h2 id="capture-title">{ko ? '화면·이미지 번역' : 'Screen & image translation'}</h2>
    <p>{ko ? '영역을 캡처하거나 이미지 파일을 가져오면 다음 단계에서 OCR과 번역을 진행합니다.' : 'Capture a region or import an image for OCR and translation.'}</p>
    <SecretModeSwitch locale={locale} value={secret} onChange={setSecret}/>
    <div className="capture-actions"><button className="primary-action" type="button" onClick={start} disabled={busy || !captureListenersReady}>{ko ? '화면 영역 선택' : 'Select screen region'}</button><button type="button" onClick={choose} disabled={busy}>{ko ? '이미지 파일 열기' : 'Open image file'}</button></div>
    {busy && <div className="capture-progress" role="status"><progress max="100" value={progress?.percent ?? 5} /><span>{ko ? '이미지를 처리하고 있습니다…' : 'Processing image…'}</span>{result && <button type="button" onClick={() => void cancelImageTranslation(result.jobId)}>{ko ? '취소' : 'Cancel'}</button>}</div>}
    {error && <p role="alert">{{
      capture: ko ? `화면 캡처를 시작할 수 없습니다. 지원 코드: ${error.supportCode}` : `Could not start screen capture. Support code: ${error.supportCode}`,
      translation: ko ? `이미지 번역을 완료할 수 없습니다. 지원 코드: ${error.supportCode}` : `Could not complete image translation. Support code: ${error.supportCode}`,
      import: ko ? `이미지를 가져올 수 없습니다. 지원 코드: ${error.supportCode}` : `Could not import image. Support code: ${error.supportCode}`,
    }[error.stage]}</p>}
    {result?.status === 'sourceReady' && <div className="capture-source-ready" role="status"><strong>{ko ? '이미지가 준비되었습니다.' : 'Image is ready.'}</strong><span>{result.imageWidth} × {result.imageHeight}</span><button className="primary-action" type="button" onClick={() => void translate()} disabled={busy}>{ko ? 'OCR 및 번역 시작' : 'Start OCR & translation'}</button></div>}
    {result?.status === 'rendered' && <CaptureResult result={result} locale={locale} onChange={setResult} onRetry={() => void translate()} />}
  </section>;
}
