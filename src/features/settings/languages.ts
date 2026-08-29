export type LanguageLabelLocale = 'ko' | 'en';

export interface SupportedLanguage {
  code: string;
  ko: string;
  en: string;
}

export const SUPPORTED_LANGUAGES: readonly SupportedLanguage[] = [
  { code: 'ko', ko: '한국어', en: 'Korean' },
  { code: 'en', ko: '영어', en: 'English' },
  { code: 'ja', ko: '일본어', en: 'Japanese' },
  { code: 'zh-CN', ko: '중국어(간체)', en: 'Chinese (Simplified)' },
  { code: 'zh-TW', ko: '중국어(번체)', en: 'Chinese (Traditional)' },
  { code: 'es', ko: '스페인어', en: 'Spanish' },
  { code: 'fr', ko: '프랑스어', en: 'French' },
  { code: 'de', ko: '독일어', en: 'German' },
  { code: 'it', ko: '이탈리아어', en: 'Italian' },
  { code: 'pt', ko: '포르투갈어', en: 'Portuguese' },
  { code: 'ru', ko: '러시아어', en: 'Russian' },
  { code: 'ar', ko: '아랍어', en: 'Arabic' },
  { code: 'hi', ko: '힌디어', en: 'Hindi' },
  { code: 'vi', ko: '베트남어', en: 'Vietnamese' },
  { code: 'th', ko: '태국어', en: 'Thai' },
  { code: 'id', ko: '인도네시아어', en: 'Indonesian' },
  { code: 'tr', ko: '터키어', en: 'Turkish' },
  { code: 'pl', ko: '폴란드어', en: 'Polish' },
  { code: 'nl', ko: '네덜란드어', en: 'Dutch' },
  { code: 'sv', ko: '스웨덴어', en: 'Swedish' },
  { code: 'da', ko: '덴마크어', en: 'Danish' },
  { code: 'no', ko: '노르웨이어', en: 'Norwegian' },
  { code: 'fi', ko: '핀란드어', en: 'Finnish' },
  { code: 'cs', ko: '체코어', en: 'Czech' },
  { code: 'uk', ko: '우크라이나어', en: 'Ukrainian' },
  { code: 'he', ko: '히브리어', en: 'Hebrew' },
] as const;

export function languageLabel(language: SupportedLanguage, locale: LanguageLabelLocale) {
  return language[locale];
}
