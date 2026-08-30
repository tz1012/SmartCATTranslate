export type DocumentFormat = 'docx' | 'pptx' | 'xlsx' | 'pdf';
export type DocumentOptions = { includeComments: boolean; includeNotes: boolean; includeHidden: boolean; wrapText: boolean; targetLanguage: string; sourceLanguage: string | null; profileId: string | null; model: string | null; pdfForceOcr: boolean; pdfFit: boolean; preserveAnnotations: boolean; outputDirectory: string | null };
export type DocumentManifest = { format: DocumentFormat; fileName: string; segmentCount: number; partCount: number; sourceHash: string; pageCount: number; pageKinds: string[]; hasSignatures: boolean; hasForms: boolean; hasAnnotations: boolean };
export type ChosenDocument = { sourcePath: string; manifest: DocumentManifest };
export type DocumentWarning = { code: string; location?: string; message: string };
export type DocumentReport = { jobId: string; format: DocumentFormat; outputPath: string; outputName: string; translatedSegments: number; warnings: DocumentWarning[]; publishable: boolean; resumedFromStage?: string };
export type DocumentProgress = { jobId: string; stage: 'inspect' | 'translate' | 'validate'; completed: number; total: number };
