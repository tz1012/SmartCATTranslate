export type Quality = 'fast' | 'balanced' | 'precise';

export type Tone = 'natural' | 'literal' | 'formal' | 'casual';

export type TranslationMode = 'translate' | 'rewrite';

export interface TranslationProfile {
  sourceLanguage: string | null;
  targetLanguage: string;
  quality: Quality;
  tone: Tone;
  protectedTerms: string[];
}

export interface TranslationRequest {
  text: string;
  profile: TranslationProfile;
  mode: TranslationMode;
  secret: boolean;
}

export interface TranslationResult {
  translatedText: string;
  detectedLanguage: string | null;
}

export type TranslationError = 'toolUseRejected';

export interface AuditEvent {
  kind: string;
  outcome: string;
  detail: string;
}
