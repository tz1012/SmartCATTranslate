import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { cancelScreenCapture, completeScreenCapture, focusCaptureWindow, getCaptureOverlay, updateScreenSelection } from './captureApi';
import type { CaptureSelection, OverlayDescriptor, PixelRect } from './types';

const MIN_SIZE = 8;
const captureSupportCodePattern = /^(?:(?:capture|screen_capture)_[a-z_]{1,48}|invalid_capture_selection)$/;

function captureSupportCode(reason: unknown): string {
  const candidate = typeof reason === 'string' ? reason : reason instanceof Error ? reason.message : '';
  return captureSupportCodePattern.test(candidate) ? candidate : 'screen_capture_completion_failed';
}

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.min(maximum, Math.max(minimum, value));
}

function rectFromPoints(start: [number, number], end: [number, number]): PixelRect {
  const x = Math.min(start[0], end[0]);
  const y = Math.min(start[1], end[1]);
  return { x, y, width: Math.abs(end[0] - start[0]), height: Math.abs(end[1] - start[1]) };
}

export function CaptureOverlay() {
  const params = useMemo(() => new URLSearchParams(window.location.search), []);
  const sessionId = params.get('session') ?? '';
  const monitorId = params.get('monitor') ?? '';
  const ko = params.get('locale') !== 'en';
  const [descriptor, setDescriptor] = useState<OverlayDescriptor>();
  const [selection, setSelection] = useState<CaptureSelection>();
  const [error, setError] = useState<string>();
  const [confirming, setConfirming] = useState(false);
  const confirmingRef = useRef(false);
  const dragStart = useRef<[number, number] | undefined>(undefined);
  const overlayRef = useRef<HTMLElement>(null);

  useEffect(() => {
    void getCaptureOverlay(sessionId, monitorId).then(setDescriptor).catch(() => setError('capture_overlay_unavailable'));
    let disposed = false;
    let stop: (() => void) | undefined;
    void listen<{ sessionId: string; selection: CaptureSelection }>('capture-selection-updated', (event) => {
      if (event.payload.sessionId === sessionId) setSelection(event.payload.selection);
    }).then((unlisten) => { if (disposed) unlisten(); else stop = unlisten; });
    return () => { disposed = true; stop?.(); };
  }, [monitorId, sessionId]);

  const physicalPoint = useCallback((clientX: number, clientY: number): [number, number] => {
    const overlay = overlayRef.current;
    if (!descriptor || !overlay) return [0, 0];
    const bounds = overlay.getBoundingClientRect();
    const monitor = descriptor.monitor.physicalBounds;
    if (bounds.width <= 0 || bounds.height <= 0) return [monitor.x, monitor.y];
    return [
      Math.round(monitor.x + ((clientX - bounds.left) / bounds.width) * monitor.width),
      Math.round(monitor.y + ((clientY - bounds.top) / bounds.height) * monitor.height),
    ];
  }, [descriptor]);

  const publish = useCallback((next: CaptureSelection) => {
    setSelection(next);
    void updateScreenSelection(sessionId, next);
  }, [sessionId]);

  const cancel = useCallback(() => { void cancelScreenCapture(sessionId); }, [sessionId]);
  const confirm = useCallback(async () => {
    if (confirmingRef.current || !selection || selection.globalPhysical.width < MIN_SIZE || selection.globalPhysical.height < MIN_SIZE) return;
    confirmingRef.current = true;
    setConfirming(true);
    setError(undefined);
    try {
      await completeScreenCapture(sessionId, selection);
    } catch (reason) {
      setError(captureSupportCode(reason));
    } finally {
      confirmingRef.current = false;
      setConfirming(false);
    }
  }, [selection, sessionId]);

  useEffect(() => {
    if (descriptor) {
      void focusCaptureWindow().catch(() => undefined);
      overlayRef.current?.focus({ preventScroll: true });
    }
  }, [descriptor]);

  useEffect(() => {
    const key = (event: KeyboardEvent) => {
      if (event.repeat) return;
      if (event.key === 'Escape' || event.key === 'Esc') { event.preventDefault(); cancel(); return; }
      if (event.key === 'Enter' || event.code === 'NumpadEnter') { event.preventDefault(); void confirm(); return; }
      if (!selection || !['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(event.key)) return;
      event.preventDefault();
      const step = event.shiftKey ? 10 : 1;
      const deltaX = event.key === 'ArrowLeft' ? -step : event.key === 'ArrowRight' ? step : 0;
      const deltaY = event.key === 'ArrowUp' ? -step : event.key === 'ArrowDown' ? step : 0;
      const rect = { ...selection.globalPhysical };
      if (event.altKey) {
        rect.width = Math.max(MIN_SIZE, rect.width + deltaX);
        rect.height = Math.max(MIN_SIZE, rect.height + deltaY);
      } else { rect.x += deltaX; rect.y += deltaY; }
      publish({ globalPhysical: rect });
    };
    document.addEventListener('keydown', key, true);
    return () => document.removeEventListener('keydown', key, true);
  }, [cancel, confirm, publish, selection]);

  if (error && !descriptor) return <div className="capture-overlay-error" role="alert">{ko ? '화면 캡처를 시작할 수 없습니다.' : 'Screen capture could not start.'}</div>;
  if (!descriptor) return <div className="capture-overlay-loading" role="status">{ko ? '캡처 준비 중…' : 'Preparing capture…'}</div>;
  const monitor = descriptor.monitor.physicalBounds;
  const selectionLeft = selection ? clamp(selection.globalPhysical.x, monitor.x, monitor.x + monitor.width) : 0;
  const selectionTop = selection ? clamp(selection.globalPhysical.y, monitor.y, monitor.y + monitor.height) : 0;
  const selectionRight = selection ? clamp(selection.globalPhysical.x + selection.globalPhysical.width, monitor.x, monitor.x + monitor.width) : 0;
  const selectionBottom = selection ? clamp(selection.globalPhysical.y + selection.globalPhysical.height, monitor.y, monitor.y + monitor.height) : 0;
  const local = selection ? {
    left: `${((selectionLeft - monitor.x) / monitor.width) * 100}%`,
    top: `${((selectionTop - monitor.y) / monitor.height) * 100}%`,
    width: `${((selectionRight - selectionLeft) / monitor.width) * 100}%`,
    height: `${((selectionBottom - selectionTop) / monitor.height) * 100}%`,
  } : undefined;

  return (
    <main
      ref={overlayRef}
      tabIndex={-1}
      className="capture-overlay"
      aria-label={ko ? '번역할 화면 영역 선택' : 'Select screen region to translate'}
      style={{ backgroundImage: `url(${descriptor.backgroundDataUrl})` }}
      onPointerDown={(event) => {
        void focusCaptureWindow().catch(() => undefined);
        event.currentTarget.focus({ preventScroll: true });
        event.currentTarget.setPointerCapture(event.pointerId);
        dragStart.current = physicalPoint(event.clientX, event.clientY);
      }}
      onPointerMove={(event) => {
        if (!dragStart.current || !event.currentTarget.hasPointerCapture(event.pointerId)) return;
        publish({ globalPhysical: rectFromPoints(dragStart.current, physicalPoint(event.clientX, event.clientY)) });
      }}
      onPointerUp={(event) => {
        if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
        dragStart.current = undefined;
      }}
    >
      <div className="capture-overlay-shade" aria-hidden="true" />
      {local && <div className="capture-selection" style={local} aria-hidden="true" />}
      <section className="capture-overlay-help" role="status">
        <strong>{ko ? '번역할 영역을 드래그하세요' : 'Drag the region to translate'}</strong>
        <span>{ko ? 'Enter 확인 · Esc 취소 · 방향키 이동 · Alt+방향키 크기 조절' : 'Enter confirm · Esc cancel · Arrow keys move · Alt+Arrow keys resize'}</span>
        {error && <span role="alert">{error}</span>}
        <div className="capture-overlay-actions">
          <button type="button" onPointerDown={(event) => event.stopPropagation()} onClick={() => void confirm()} disabled={confirming || !selection || selection.globalPhysical.width < MIN_SIZE || selection.globalPhysical.height < MIN_SIZE}>{ko ? '확인' : 'Confirm'}</button>
          <button type="button" onPointerDown={(event) => event.stopPropagation()} onClick={cancel}>{ko ? '취소' : 'Cancel'}</button>
        </div>
      </section>
    </main>
  );
}
