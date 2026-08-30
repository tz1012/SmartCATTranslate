export type CoordinateSpace = 'physicalPixels' | 'logicalPoints' | 'normalized';
export type PixelRect = { x: number; y: number; width: number; height: number };
export type LogicalRect = { x: number; y: number; width: number; height: number };
export type NormalizedRect = { x: number; y: number; width: number; height: number };
export type TextDirection = 'leftToRight' | 'rightToLeft' | 'topToBottom';

export type OcrLine = {
  id: string;
  text: string;
  bounds: NormalizedRect;
  confidence: number;
  angleDegrees: number;
  direction: TextDirection;
};

export type OcrDocument = {
  imageWidth: number;
  imageHeight: number;
  lines: OcrLine[];
  sourceLanguage?: string;
};

export type TranslatedBlock = {
  sourceIds: string[];
  sourceText: string;
  translatedText: string;
  bounds: NormalizedRect;
  confidence: number;
};

export type CaptureJobResult = {
  jobId: string;
  status: 'sourceReady' | 'ocrReady' | 'translated' | 'rendered';
  imageWidth: number;
  imageHeight: number;
  ocr?: OcrDocument;
  translatedBlocks: TranslatedBlock[];
  warnings: string[];
};

export type MonitorInfo = {
  id: string;
  name: string;
  physicalBounds: PixelRect;
  logicalBounds: LogicalRect;
  scaleFactor: number;
  primary: boolean;
};

export type CaptureSelection = { globalPhysical: PixelRect };
export type OverlayDescriptor = { sessionId: string; monitor: MonitorInfo; backgroundDataUrl: string };
