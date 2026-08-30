export type DocumentFormat = 'docx' | 'pptx' | 'xlsx' | 'pdf';
export type DocumentOptions = { includeComments: boolean; includeNotes: boolean; includeHidden: boolean; wrapText: boolean; targetLanguage: string; sourceLanguage: string | null; profileId: string | null; model: string | null; quality: 'fast' | 'balanced' | 'precise' | null; pdfForceOcr: boolean; pdfFit: boolean; preserveAnnotations: boolean; secret: boolean; outputDirectory: string | null };
export type DocumentManifest = { format: DocumentFormat; fileName: string; segmentCount: number; partCount: number; sourceHash: string; pageCount: number; pageKinds: string[]; hasSignatures: boolean; hasForms: boolean; hasAnnotations: boolean };
export type ChosenDocument = { sourcePath: string; manifest: DocumentManifest };
export type DocumentWarning = { code: string; location?: string; message: string };
export type DocumentReport = { jobId: string; format: DocumentFormat; outputPath: string; outputName: string; translatedSegments: number; warnings: DocumentWarning[]; publishable: boolean; resumedFromStage?: string };
export type DocumentProgress = { jobId: string; stage: 'inspect' | 'extract' | 'ocr' | 'translate' | 'reflow' | 'save' | 'validate' | 'completed'; unitId?: string; completed: number; total: number };
export type DocumentCheckpoint = { sourceFingerprint: string; stage: DocumentProgress['stage']; stableUnitId: string; completed: number; total: number; completedBatchCursor: number; rasterRefs: string[]; translatedResultRefs: string[] };
export type DocumentResumeRequest = { recordId: string; optionHash: string };
export type PreparedDocumentRecovery = { recordId:string;sourcePath:string;options:DocumentOptions;optionHash:string };
export type DocumentResultPreview =
  | { kind: 'pdfPage'; location: string; label: string; imageDataUrl: string; width: number; height: number }
  | { kind: 'pptxSlide'; location: string; label: string; width: number; height: number; focusTextOrdinal?: number; shapes: Array<{ id: string; name: string; text: string; x: number; y: number; width: number; height: number; textStart: number; textEnd: number }> }
  | { kind: 'xlsxCell'; location: string; label: string; focusCell: string; columns: string[]; rows: Array<Array<{ reference: string; value: string; focused: boolean }>> }
  | { kind: 'docxContext'; location: string; label: string; lines: Array<{ ordinal: number; text: string; focused: boolean }> };
export type DocumentJobEvent =
  | { type: 'progress'; jobId: string; checkpoint: DocumentCheckpoint }
  | { type: 'warning'; jobId: string; warning: DocumentWarning }
  | { type: 'completed'; jobId: string; report: DocumentReport }
  | { type: 'failed'; jobId: string; code: string; location?: string }
  | { type: 'inspect'; jobId: string; manifest: DocumentManifest };
export type DocumentProfileOption = { id: string; name: string };
