import { useCallback, useEffect, useRef, useState } from 'react';
import type { TranslationRequest } from '../../lib/types';
import {
  cancelTranslation,
  onTranslationEvent,
  translateText,
  type TranslationEvent,
} from './translationApi';

export type TranslationJobState =
  | { status: 'idle'; text: '' }
  | { status: 'running'; jobId: string; text: string }
  | { status: 'completed'; jobId: string; text: string }
  | { status: 'failed'; jobId?: string; text: string; message: string; pendingCleanup?: boolean };

export type TranslationListenerState = 'connecting' | 'ready' | 'failed';

export function useTranslationJob() {
  const [state, setState] = useState<TranslationJobState>({ status: 'idle', text: '' });
  const [detectedLanguage, setDetectedLanguage] = useState<string | null>(null);
  const [listenerState, setListenerState] = useState<TranslationListenerState>('connecting');
  const mounted = useRef(false);
  const generation = useRef(0);
  const starting = useRef(false);
  const activeJobId = useRef<string | null>(null);
  const pendingEvents = useRef<TranslationEvent[]>([]);
  const cancelSent = useRef(false);
  const cancelRequested = useRef(false);
  const listenerReady = useRef(false);

  const applyEvent = useCallback((event: TranslationEvent, expectedGeneration: number) => {
    if (!mounted.current || expectedGeneration !== generation.current) return;
    if (activeJobId.current !== event.jobId) return;
    if (event.type === 'delta') {
      setState((current) => current.status === 'running' && current.jobId === event.jobId
        ? { ...current, text: current.text + event.text }
        : current);
      return;
    }
    activeJobId.current = null;
    cancelSent.current = false;
    cancelRequested.current = false;
    if (event.type === 'completed') {
      setDetectedLanguage(event.result.detectedLanguage);
      setState({ status: 'completed', jobId: event.jobId, text: event.result.translatedText });
    } else {
      setState({ status: 'failed', jobId: event.jobId, text: '', message: event.code });
    }
  }, []);

  useEffect(() => {
    mounted.current = true;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void onTranslationEvent((event) => {
      if (starting.current && activeJobId.current === null) {
        if (pendingEvents.current.length < 256) pendingEvents.current.push(event);
        return;
      }
      applyEvent(event, generation.current);
    }).then((stop) => {
      if (disposed) stop();
      else {
        unlisten = stop;
        listenerReady.current = true;
        setListenerState('ready');
      }
    }).catch(() => {
      if (!disposed && mounted.current) {
        listenerReady.current = false;
        setListenerState('failed');
        setState({ status: 'failed', text: '', message: 'translation_listener_unavailable' });
      }
    });
    return () => {
      disposed = true;
      mounted.current = false;
      listenerReady.current = false;
      generation.current += 1;
      unlisten?.();
      const jobId = activeJobId.current;
      activeJobId.current = null;
      if (jobId && !cancelSent.current) void cancelTranslation(jobId);
    };
  }, [applyEvent]);

  const start = useCallback(async (request: TranslationRequest) => {
    if (!listenerReady.current || starting.current || activeJobId.current !== null) return;
    const thisGeneration = ++generation.current;
    starting.current = true;
    cancelSent.current = false;
    cancelRequested.current = false;
    pendingEvents.current = [];
    setDetectedLanguage(null);
    setState({ status: 'running', jobId: '', text: '' });
    try {
      const jobId = await translateText(request);
      if (!mounted.current || thisGeneration !== generation.current) {
        await cancelTranslation(jobId);
        return;
      }
      activeJobId.current = jobId;
      setState({ status: 'running', jobId, text: '' });
      if (cancelRequested.current) {
        cancelSent.current = true;
        const cancelled = await cancelTranslation(jobId);
        if (!cancelled) {
          cancelSent.current = false;
          setState({ status: 'failed', jobId, text: '', message: 'translation_cancel_failed', pendingCleanup: true });
        }
      }
      const earlyEvents = pendingEvents.current;
      pendingEvents.current = [];
      for (const event of earlyEvents) applyEvent(event, thisGeneration);
    } catch {
      if (mounted.current && thisGeneration === generation.current) {
        if (activeJobId.current) {
          cancelSent.current = false;
          setState({ status: 'failed', jobId: activeJobId.current, text: '', message: 'translation_cancel_failed', pendingCleanup: true });
        } else {
          setState({ status: 'failed', text: '', message: 'translation_start_failed' });
        }
      }
    } finally {
      if (thisGeneration === generation.current) starting.current = false;
    }
  }, [applyEvent]);

  const cancel = useCallback(async () => {
    const jobId = activeJobId.current;
    if (!jobId) {
      if (starting.current) cancelRequested.current = true;
      return;
    }
    if (cancelSent.current) return;
    cancelSent.current = true;
    try {
      const cancelled = await cancelTranslation(jobId);
      if (!cancelled) throw new Error('cancel rejected');
    } catch {
      if (mounted.current && activeJobId.current === jobId) {
        cancelSent.current = false;
        setState({ status: 'failed', jobId, text: '', message: 'translation_cancel_failed', pendingCleanup: true });
      }
    }
  }, []);

  const reset = useCallback(() => {
    if (activeJobId.current !== null || starting.current) return;
    generation.current += 1;
    setDetectedLanguage(null);
    setState({ status: 'idle', text: '' });
  }, []);

  return { state, detectedLanguage, listenerState, start, cancel, reset };
}
