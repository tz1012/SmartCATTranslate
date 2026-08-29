import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { TranslationRequest, TranslationResult } from '../../lib/types';

export type TranslationEvent =
  | { type: 'delta'; jobId: string; text: string }
  | { type: 'completed'; jobId: string; result: TranslationResult }
  | { type: 'failed'; jobId: string; code: string; message: string };

export function translateText(request: TranslationRequest): Promise<string> {
  return invoke<string>('translate_text', { request });
}

export function cancelTranslation(jobId: string): Promise<boolean> {
  return invoke<boolean>('cancel_translation', { jobId });
}

export function onTranslationEvent(
  handler: (event: TranslationEvent) => void,
): Promise<UnlistenFn> {
  return listen<TranslationEvent>('translation-event', (event) => handler(event.payload));
}
