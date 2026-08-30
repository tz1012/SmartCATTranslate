import { invoke } from '@tauri-apps/api/core';

export type HistoryRecord = {
  id: string;
  createdAt: string;
  kind: string;
  sourceLanguage: string | null;
  targetLanguage: string;
  source: string;
  result: string;
  displayName: string | null;
  warningCount: number;
};
export type HistoryPage = { records: HistoryRecord[]; nextCursor: string | null };
export type NewHistoryRecord = Omit<HistoryRecord, 'id' | 'createdAt'> & { secret: boolean };
export type RecoverableJob = {
  recordId: string;
  displayName: string;
  kind: string;
  stage: string;
  completed: number;
  total: number;
  createdAt: string;
  canResume: boolean;
  disabledReason: string | null;
  secret: boolean;
};
export type PreparedDocumentRecovery = {
  recordId: string;
  sourcePath: string;
  options: import('../documents/types').DocumentOptions;
  optionHash: string;
};

export const saveHistoryRecord = (record: NewHistoryRecord) =>
  invoke<string | null>('save_history_record', { record });
export const listHistory = (limit = 50, cursor: string | null = null) =>
  invoke<HistoryPage>('list_history', { limit, cursor });
export const deleteHistory = (id: string) => invoke<boolean>('delete_history', { id });
export const deleteAllHistory = () => invoke<number>('delete_all_history');
export const listRecoverableJobs = () => invoke<RecoverableJob[]>('list_recoverable_jobs');
export const prepareDocumentRecovery = (recordId: string) =>
  invoke<PreparedDocumentRecovery>('prepare_document_recovery', { recordId });
export const deleteRecoveryJob = (recordId: string) =>
  invoke<boolean>('delete_recovery_job', { recordId });
