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
  const listenerStatus = useRef<TranslationListenerState>('connecting');
  const listenerAttempt = useRef(0);
  const unlistenTranslation = useRef<(() => void) | undefined>(undefined);

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

  const registerListener = useCallback(() => {
    const attempt = ++listenerAttempt.current;
    void onTranslationEvent((event) => {
      if (starting.current && activeJobId.current === null) {
        if (pendingEvents.current.length < 256) pendingEvents.current.push(event);
        return;
      }
      applyEvent(event, generation.current);
    }).then((stop) => {
      if (!mounted.current || attempt !== listenerAttempt.current) {
        stop();
        return;
      }
      unlistenTranslation.current = stop;
      listenerReady.current = true;
      listenerStatus.current = 'ready';
      setListenerState('ready');
    }).catch(() => {
      if (!mounted.current || attempt !== listenerAttempt.current) return;
      listenerReady.current = false;
      listenerStatus.current = 'failed';
      setListenerState('failed');
      setState({ status: 'failed', text: '', message: 'translation_listener_unavailable' });
    });
  }, [applyEvent]);

  useEffect(() => {
    mounted.current = true;
    registerListener();
    return () => {
      mounted.current = false;
      listenerReady.current = false;
      listenerAttempt.current += 1;
      generation.current += 1;
      unlistenTranslation.current?.();
      unlistenTranslation.current = undefined;
      const jobId = activeJobId.current;
      activeJobId.current = null;
      if (jobId && !cancelSent.current) void cancelTranslation(jobId);
    };
  }, [registerListener]);

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
    } catch (error) {
      if (mounted.current && thisGeneration === generation.current) {
        if (activeJobId.current) {
          cancelSent.current = false;
          setState({ status: 'failed', jobId: activeJobId.current, text: '', message: 'translation_cancel_failed', pendingCleanup: true });
        } else {
          const code = typeof error === 'string' && /^[a-z][a-z0-9_]{0,95}$/.test(error)
            ? error
            : 'translation_start_failed';
          setState({ status: 'failed', text: '', message: code });
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

  const retryListener = useCallback(() => {
    if (!mounted.current || listenerStatus.current !== 'failed' || starting.current || activeJobId.current !== null) return;
    listenerReady.current = false;
    listenerStatus.current = 'connecting';
    setListenerState('connecting');
    setState((current) => current.status === 'failed' && current.message === 'translation_listener_unavailable'
      ? { status: 'idle', text: '' }
      : current);
    registerListener();
  }, [registerListener]);

  return { state, detectedLanguage, listenerState, start, cancel, reset, retryListener };
}
