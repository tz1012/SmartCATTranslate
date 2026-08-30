import { invoke } from '@tauri-apps/api/core';
import type { ChosenDocument, DocumentOptions, DocumentReport } from './types';
export const chooseDocument = (options: DocumentOptions) => invoke<ChosenDocument | null>('choose_document', { options });
export const translateDocument = (jobId: string, sourcePath: string, options: DocumentOptions) => invoke<DocumentReport>('translate_document', { jobId, sourcePath, options });
export const cancelDocumentTranslation = (jobId: string) => invoke<boolean>('cancel_document_translation', { jobId });
export const openDocumentResult = (path: string) => invoke<void>('open_document_result', { path });
export const openDocumentFolder = (path: string) => invoke<void>('open_document_folder', { path });
export const chooseDocumentOutputDirectory = () => invoke<string | null>('choose_document_output_directory');
