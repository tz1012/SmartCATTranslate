import { useEffect, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { cancelImageTranslation, chooseImage, startScreenCapture, translateImage } from './captureApi';
import type { CaptureJobResult, CaptureProgress } from './types';
import { CaptureResult } from './CaptureResult';
import { SecretModeSwitch,useSecretMode } from '../history/secretMode';

export function CaptureWorkspace({ locale }: { locale: 'ko' | 'en' }) {
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<CaptureJobResult>();
  const [error, setError] = useState<string>();
  const [progress, setProgress] = useState<CaptureProgress>();
  const ko = locale === 'ko';
  const [secret,setSecret]=useSecretMode();
  useEffect(() => {
    let disposed = false; let stop: (() => void) | undefined;
    void listen<CaptureJobResult>('capture-source-ready', (event) => { setResult(event.payload); setBusy(false); })
      .then((unlisten) => { if (disposed) unlisten(); else stop = unlisten; });
    return () => { disposed = true; stop?.(); };
  }, []);
  useEffect(() => { let disposed = false; let stop: (() => void) | undefined; void listen<CaptureProgress>('capture-progress', (event) => { if (!result || event.payload.jobId === result.jobId) setProgress(event.payload); }).then((unlisten) => { if (disposed) unlisten(); else stop = unlisten; }); return () => { disposed = true; stop?.(); }; }, [result]);
  const start = async () => {
    setBusy(true); setError(undefined);
    try { await startScreenCapture(); }
    catch (reason) { setBusy(false); setError(String(reason)); }
  };
  const translate = async () => {
    if (!result) return; setBusy(true); setError(undefined); setProgress({ jobId: result.jobId, stage: 'ocr', percent: 5 });
    try { setResult(await translateImage(result.jobId, ['ko', 'en'],secret)); }
    catch (reason) { setError(String(reason)); }
    finally { setBusy(false); }
  };
  const choose = async () => {
    setBusy(true); setError(undefined);
    try { const imported = await chooseImage(); if (imported) setResult(imported); }
    catch (reason) { setError(String(reason)); }
    finally { setBusy(false); }
  };
  return <section className="capture-workspace" aria-labelledby="capture-title">
    <h2 id="capture-title">{ko ? '화면·이미지 번역' : 'Screen & image translation'}</h2>
    <p>{ko ? '영역을 캡처하거나 이미지 파일을 가져오면 다음 단계에서 OCR과 번역을 진행합니다.' : 'Capture a region or import an image for OCR and translation.'}</p>
    <SecretModeSwitch locale={locale} value={secret} onChange={setSecret}/>
    <div className="capture-actions"><button className="primary-action" type="button" onClick={start} disabled={busy}>{ko ? '화면 영역 선택' : 'Select screen region'}</button><button type="button" onClick={choose} disabled={busy}>{ko ? '이미지 파일 열기' : 'Open image file'}</button></div>
    {busy && <div className="capture-progress" role="status"><progress max="100" value={progress?.percent ?? 5} /><span>{ko ? '이미지를 처리하고 있습니다…' : 'Processing image…'}</span>{result && <button type="button" onClick={() => void cancelImageTranslation(result.jobId)}>{ko ? '취소' : 'Cancel'}</button>}</div>}
    {error && <p role="alert">{ko ? `캡처를 시작하지 못했습니다: ${error}` : `Could not start capture: ${error}`}</p>}
    {result?.status === 'sourceReady' && <div className="capture-source-ready" role="status"><strong>{ko ? '이미지가 준비되었습니다.' : 'Image is ready.'}</strong><span>{result.imageWidth} × {result.imageHeight}</span><button className="primary-action" type="button" onClick={() => void translate()} disabled={busy}>{ko ? 'OCR 및 번역 시작' : 'Start OCR & translation'}</button></div>}
    {result?.status === 'rendered' && <CaptureResult result={result} locale={locale} onChange={setResult} onRetry={() => void translate()} />}
  </section>;
}
