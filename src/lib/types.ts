export type Quality = 'fast' | 'balanced' | 'precise';

export type Tone = 'natural' | 'literal' | 'formal' | 'casual';

export type TranslationMode = 'translate' | 'rewrite';

export type Field = 'general' | 'technical' | 'legal' | 'medical' | 'business';

export interface GlossaryMapping {
  sourceTerm: string;
  targetTerm: string;
}

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
  field: Field;
  glossary: GlossaryMapping[];
  mode: TranslationMode;
  secret: boolean;
}

export interface TranslationResult {
  translatedText: string;
  detectedLanguage: string | null;
}

export type CompletedTextTranslation = {
  id: string;
  source: string;
  translation: string;
};

export type TranslationError = 'toolUseRejected';

export interface AuditEvent {
  kind: string;
  outcome: string;
  detail: string;
}
