import type { TranslationProfile } from '../../lib/types';
import type { AppSettings, SavedProfile } from './SettingsView';

export const DEFAULT_TRANSLATION_PROFILE: TranslationProfile = {
  sourceLanguage: null,
  targetLanguage: 'ko',
  quality: 'balanced',
  tone: 'natural',
  protectedTerms: [],
};

export function resolveDefaultProfile(settings: AppSettings): SavedProfile | null {
  return settings.profiles.find((profile) => profile.id === settings.defaultProfileId) ?? null;
}
