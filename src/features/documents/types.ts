export type DocumentFormat = 'docx' | 'pptx' | 'xlsx' | 'pdf';
export type DocumentOptions = { includeComments: boolean; includeNotes: boolean; includeHidden: boolean; wrapText: boolean; targetLanguage: string; sourceLanguage: string | null; profileId: string | null; model: string | null; quality: 'fast' | 'balanced' | 'precise' | null; pdfForceOcr: boolean; pdfFit: boolean; preserveAnnotations: boolean; secret: boolean; outputDirectory: string | null };
export type DocumentManifest = { format: DocumentFormat; fileName: string; segmentCount: number; partCount: number; sourceHash: string; pageCount: number; pageKinds: string[]; hasSignatures: boolean; hasForms: boolean; hasAnnotations: boolean };
export type ChosenDocument = { sourcePath: string; manifest: DocumentManifest };
export type DocumentWarning = { code: string; location?: string; message: string };
export type DocumentReport = { jobId: string; format: DocumentFormat; outputPath: string; outputName: string; translatedSegments: number; warnings: DocumentWarning[]; publishable: boolean; resumedFromStage?: string };
export type DocumentProgress = { jobId: string; stage: 'inspect' | 'extract' | 'ocr' | 'translate' | 'reflow' | 'save' | 'validate' | 'completed'; unitId?: string; completed: number; total: number };
export type DocumentCheckpoint = { sourceFingerprint: string; stage: DocumentProgress['stage']; stableUnitId: string; completed: number; total: number; rasterRefs: string[]; translatedResultRefs: string[] };
export type DocumentJobEvent =
  | { type: 'progress'; jobId: string; checkpoint: DocumentCheckpoint }
  | { type: 'warning'; jobId: string; warning: DocumentWarning }
  | { type: 'completed'; jobId: string; report: DocumentReport }
  | { type: 'failed'; jobId: string; code: string; location?: string }
  | { type: 'retentionRequested'; jobId: string };
export type DocumentProfileOption = { id: string; name: string };
