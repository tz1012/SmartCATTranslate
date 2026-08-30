import { invoke } from '@tauri-apps/api/core';
import type { CaptureJobResult, CaptureSelection, MonitorInfo, OverlayDescriptor } from './types';

export type StartCaptureResult = { sessionId: string; monitors: MonitorInfo[] };

export const startScreenCapture = () => invoke<StartCaptureResult>('start_screen_capture');
export const getCaptureOverlay = (sessionId: string, monitorId: string) =>
  invoke<OverlayDescriptor>('get_capture_overlay', { sessionId, monitorId });
export const updateScreenSelection = (sessionId: string, selection: CaptureSelection) =>
  invoke<void>('update_screen_selection', { sessionId, selection });
export const completeScreenCapture = (sessionId: string, selection: CaptureSelection) =>
  invoke<CaptureJobResult>('complete_screen_capture', { sessionId, selection });
export const cancelScreenCapture = (sessionId: string) =>
  invoke<void>('cancel_screen_capture', { sessionId });
export const chooseImage = () => invoke<CaptureJobResult | null>('choose_image');
export const translateImage = (jobId: string, languageHints: string[]) => invoke<CaptureJobResult>('translate_image', { jobId, languageHints });
export const cancelImageTranslation = (jobId: string) => invoke<void>('cancel_image_translation', { jobId });
export const updateCaptureBlock = (jobId: string, blockId: string, translatedText: string, visible: boolean) =>
  invoke<CaptureJobResult>('update_capture_block', { jobId, blockId, translatedText, visible });
export const exportTranslatedImage = (jobId: string, format: 'png' | 'jpeg' | 'webp', replace = false) =>
  invoke<string | null>('export_translated_image', { jobId, format, replace });
