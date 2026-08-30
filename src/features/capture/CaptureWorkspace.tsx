import { useEffect, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { chooseImage, startScreenCapture } from './captureApi';
import type { CaptureJobResult } from './types';

export function CaptureWorkspace({ locale }: { locale: 'ko' | 'en' }) {
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<CaptureJobResult>();
  const [error, setError] = useState<string>();
  const ko = locale === 'ko';
  useEffect(() => {
    let disposed = false; let stop: (() => void) | undefined;
    void listen<CaptureJobResult>('capture-source-ready', (event) => { setResult(event.payload); setBusy(false); })
      .then((unlisten) => { if (disposed) unlisten(); else stop = unlisten; });
    return () => { disposed = true; stop?.(); };
  }, []);
  const start = async () => {
    setBusy(true); setError(undefined);
    try { await startScreenCapture(); }
    catch (reason) { setBusy(false); setError(String(reason)); }
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
    <div className="capture-actions"><button className="primary-action" type="button" onClick={start} disabled={busy}>{ko ? '화면 영역 선택' : 'Select screen region'}</button><button type="button" onClick={choose} disabled={busy}>{ko ? '이미지 파일 열기' : 'Open image file'}</button></div>
    {busy && <p role="status">{ko ? '모니터 화면을 안전하게 준비하고 있습니다…' : 'Preparing monitor snapshots…'}</p>}
    {error && <p role="alert">{ko ? `캡처를 시작하지 못했습니다: ${error}` : `Could not start capture: ${error}`}</p>}
    {result && <div className="capture-source-ready" role="status"><strong>{ko ? '이미지가 준비되었습니다.' : 'Image is ready.'}</strong><span>{result.imageWidth} × {result.imageHeight}</span><small>{ko ? '다음 OCR 단계로 전달되었습니다.' : 'Routed to the OCR stage.'}</small></div>}
  </section>;
}
