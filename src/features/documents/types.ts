export type DocumentFormat = 'docx' | 'pptx' | 'xlsx';
export type DocumentOptions = { includeComments: boolean; includeNotes: boolean; includeHidden: boolean; wrapText: boolean; targetLanguage: string };
export type DocumentManifest = { format: DocumentFormat; fileName: string; segmentCount: number; partCount: number; sourceHash: string };
export type ChosenDocument = { sourcePath: string; manifest: DocumentManifest };
export type DocumentWarning = { code: string; location?: string; message: string };
export type DocumentReport = { jobId: string; format: DocumentFormat; outputPath: string; outputName: string; translatedSegments: number; warnings: DocumentWarning[]; publishable: boolean };
export type DocumentProgress = { jobId: string; stage: 'inspect' | 'translate' | 'validate'; completed: number; total: number };
