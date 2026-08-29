import { useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { Quality, Tone, TranslationProfile } from '../../lib/types';
import { GlossaryEditor } from './GlossaryEditor';
import { languageLabel, SUPPORTED_LANGUAGES } from './languages';
import { ModelSelector, modelChoiceValue } from './ModelSelector';

export type AppLocale = 'ko' | 'en';
export type Theme = 'system' | 'light' | 'dark';
export type Field = 'general' | 'technical' | 'legal' | 'medical' | 'business';
export type CloseBehavior = 'keepInTray' | 'quit' | 'askEveryTime';
export type QuickAccessPosition = 'popup' | 'mainWindow';
export type ModelChoice = { type: 'automatic' } | { type: 'specific'; id: string };

export interface SavedProfile {
  id: string;
  name: string;
  field: Field;
  profile: TranslationProfile;
}

export interface GlossaryEntry {
  id: string;
  sourceLanguage: string;
  targetLanguage: string;
  sourceTerm: string;
  targetTerm: string;
  protectOnly: boolean;
}

export interface AvailableModel {
  id: string;
  displayName: string;
  supportedReasoningEfforts: string[];
  isDefault: boolean;
}

export interface AppSettings {
  schemaVersion: number;
  locale: AppLocale;
  theme: Theme;
  defaultProfileId: string;
  profiles: SavedProfile[];
  glossary: GlossaryEntry[];
  selectedModel: ModelChoice;
  launchAtLogin: boolean;
  closeBehavior: CloseBehavior;
  quickAccessPosition: QuickAccessPosition;
  historyRetentionDays: number;
}

const copy = {
  ko: {
    title: '설정', loading: '설정을 불러오는 중', loadError: '설정을 불러올 수 없습니다', profiles: '번역 프로필',
    addProfile: '프로필 추가', deleteProfile: '프로필 삭제', profileName: '프로필 이름', locale: '화면 언어', theme: '테마',
    source: '원문 언어', target: '대상 언어', quality: '품질', tone: '문체', field: '분야', auto: '자동 감지',
    system: '시스템', light: '밝게', dark: '어둡게', fast: '빠른', balanced: '균형', precise: '정밀', natural: '자연스럽게',
    literal: '직역', formal: '격식', casual: '일상', general: '일반', technical: '기술', legal: '법률', medical: '의학', business: '비즈니스',
    launch: '로그인할 때 실행', close: '닫기 동작', keepInTray: '트레이에 유지', quit: '앱 종료', askEveryTime: '매번 묻기',
    quickPosition: '빠른 번역 위치', popup: '작은 팝업', mainWindow: '전체 창', save: '설정 저장', saved: '저장됨', saveError: '설정을 저장할 수 없습니다',
    rewritePrompt: '문장을 개선할까요?', rewrite: '문장 개선', changeTarget: '대상 언어 변경', defaultProfile: '기본 프로필', newProfile: '새 프로필',
  },
  en: {
    title: 'Settings', loading: 'Loading settings', loadError: 'Could not load settings', profiles: 'Translation profiles',
    addProfile: 'Add profile', deleteProfile: 'Delete profile', profileName: 'Profile name', locale: 'Interface language', theme: 'Theme',
    source: 'Source language', target: 'Target language', quality: 'Quality', tone: 'Tone', field: 'Field', auto: 'Detect automatically',
    system: 'System', light: 'Light', dark: 'Dark', fast: 'Fast', balanced: 'Balanced', precise: 'Precise', natural: 'Natural',
    literal: 'Literal', formal: 'Formal', casual: 'Casual', general: 'General', technical: 'Technical', legal: 'Legal', medical: 'Medical', business: 'Business',
    launch: 'Launch at login', close: 'Close behavior', keepInTray: 'Keep in tray', quit: 'Quit app', askEveryTime: 'Ask every time',
    quickPosition: 'Quick translation position', popup: 'Small popup', mainWindow: 'Main window', save: 'Save settings', saved: 'Saved', saveError: 'Could not save settings',
    rewritePrompt: 'Improve the sentence?', rewrite: 'Improve writing', changeTarget: 'Change target language', defaultProfile: 'Default profile', newProfile: 'New profile',
  },
} as const;

function newId() {
  return globalThis.crypto?.randomUUID?.() ?? `profile-${Date.now()}`;
}

function displayProfileName(profile: SavedProfile, settings: AppSettings, locale: AppLocale) {
  return locale === 'en' && profile.id === settings.defaultProfileId && profile.name === '기본 프로필'
    ? copy.en.defaultProfile
    : profile.name;
}

export function SettingsView({
  detectedSourceLanguage,
  onRewrite,
}: {
  detectedSourceLanguage?: string;
  onRewrite?: () => void;
}) {
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [models, setModels] = useState<AvailableModel[]>([]);
  const [selectedProfileId, setSelectedProfileId] = useState('');
  const [status, setStatus] = useState('');
  const [loadFailed, setLoadFailed] = useState(false);

  useEffect(() => {
    let active = true;
    void Promise.all([
      invoke<AppSettings>('get_settings'),
      invoke<AvailableModel[]>('list_available_models').catch(() => []),
    ]).then(([loadedSettings, loadedModels]) => {
      if (!active) return;
      setSettings(loadedSettings);
      setModels(loadedModels);
      setSelectedProfileId(loadedSettings.defaultProfileId);
    }).catch(() => active && setLoadFailed(true));
    return () => { active = false; };
  }, []);

  const locale = settings?.locale ?? 'ko';
  const labels = copy[locale];
  const selectedProfile = useMemo(() => settings?.profiles.find((profile) => profile.id === selectedProfileId)
    ?? settings?.profiles.find((profile) => profile.id === settings.defaultProfileId), [settings, selectedProfileId]);

  if (loadFailed) return <section aria-label={labels.title}><p role="alert">{labels.loadError}</p></section>;
  if (!settings || !selectedProfile) return <section aria-label={labels.title}><p role="status">{labels.loading}</p></section>;

  const updateProfile = (updater: (profile: SavedProfile) => SavedProfile) => {
    setSettings((current) => current && ({
      ...current,
      profiles: current.profiles.map((profile) => profile.id === selectedProfile.id ? updater(profile) : profile),
    }));
  };
  const updateTranslationProfile = (patch: Partial<TranslationProfile>) => updateProfile((profile) => ({
    ...profile, profile: { ...profile.profile, ...patch },
  }));

  const addProfile = () => {
    const profile: SavedProfile = {
      ...selectedProfile,
      id: newId(),
      name: labels.newProfile,
      profile: { ...selectedProfile.profile, protectedTerms: [...selectedProfile.profile.protectedTerms] },
    };
    setSettings({ ...settings, profiles: [...settings.profiles, profile] });
    setSelectedProfileId(profile.id);
  };
  const deleteProfile = () => {
    if (selectedProfile.id === settings.defaultProfileId) return;
    setSettings({ ...settings, profiles: settings.profiles.filter((profile) => profile.id !== selectedProfile.id) });
    setSelectedProfileId(settings.defaultProfileId);
  };

  const save = async () => {
    const savedModel = modelChoiceValue(settings.selectedModel, models) === 'automatic'
      ? { type: 'automatic' } as const
      : settings.selectedModel;
    const candidate = { ...settings, selectedModel: savedModel };
    try {
      const saved = await invoke<AppSettings>('save_settings', { settings: candidate });
      setSettings(saved);
      setStatus(labels.saved);
    } catch {
      setStatus(labels.saveError);
    }
  };

  const rewriteSuggested = Boolean(detectedSourceLanguage
    && detectedSourceLanguage.toLowerCase() === selectedProfile.profile.targetLanguage.toLowerCase());

  return (
    <section aria-labelledby="settings-title">
      <h2 id="settings-title">{labels.title}</h2>
      <label>{labels.locale}<select value={settings.locale} onChange={(event) => setSettings({ ...settings, locale: event.target.value as AppLocale })}>
        <option value="ko">{locale === 'ko' ? '한국어' : 'Korean'}</option><option value="en">English</option>
      </select></label>
      <label>{labels.theme}<select value={settings.theme} onChange={(event) => setSettings({ ...settings, theme: event.target.value as Theme })}>
        <option value="system">{labels.system}</option><option value="light">{labels.light}</option><option value="dark">{labels.dark}</option>
      </select></label>

      <fieldset>
        <legend>{labels.profiles}</legend>
        <select aria-label={labels.profiles} value={selectedProfile.id} onChange={(event) => setSelectedProfileId(event.target.value)}>
          {settings.profiles.map((profile) => <option key={profile.id} value={profile.id}>{displayProfileName(profile, settings, locale)}</option>)}
        </select>
        <button type="button" onClick={addProfile}>{labels.addProfile}</button>
        <button type="button" disabled={selectedProfile.id === settings.defaultProfileId} onClick={deleteProfile}>{labels.deleteProfile}</button>
        <label>{labels.profileName}<input value={displayProfileName(selectedProfile, settings, locale)} onChange={(event) => updateProfile((profile) => ({ ...profile, name: event.target.value }))} /></label>
        <label>{labels.source}<select value={selectedProfile.profile.sourceLanguage ?? 'auto'} onChange={(event) => updateTranslationProfile({ sourceLanguage: event.target.value === 'auto' ? null : event.target.value })}>
          <option value="auto">{labels.auto}</option>
          {SUPPORTED_LANGUAGES.map((language) => <option key={language.code} value={language.code}>{languageLabel(language, locale)}</option>)}
        </select></label>
        <label>{labels.target}<select value={selectedProfile.profile.targetLanguage} onChange={(event) => updateTranslationProfile({ targetLanguage: event.target.value })}>
          {SUPPORTED_LANGUAGES.map((language) => <option key={language.code} value={language.code}>{languageLabel(language, locale)}</option>)}
        </select></label>
        <label>{labels.quality}<select value={selectedProfile.profile.quality} onChange={(event) => updateTranslationProfile({ quality: event.target.value as Quality })}>
          <option value="fast">{labels.fast}</option><option value="balanced">{labels.balanced}</option><option value="precise">{labels.precise}</option>
        </select></label>
        <label>{labels.tone}<select value={selectedProfile.profile.tone} onChange={(event) => updateTranslationProfile({ tone: event.target.value as Tone })}>
          <option value="natural">{labels.natural}</option><option value="literal">{labels.literal}</option><option value="formal">{labels.formal}</option><option value="casual">{labels.casual}</option>
        </select></label>
        <label>{labels.field}<select value={selectedProfile.field} onChange={(event) => updateProfile((profile) => ({ ...profile, field: event.target.value as Field }))}>
          <option value="general">{labels.general}</option><option value="technical">{labels.technical}</option><option value="legal">{labels.legal}</option><option value="medical">{labels.medical}</option><option value="business">{labels.business}</option>
        </select></label>
      </fieldset>

      {rewriteSuggested && <aside aria-live="polite"><p>{labels.rewritePrompt}</p><button type="button" onClick={onRewrite}>{labels.rewrite}</button><button type="button" onClick={() => updateTranslationProfile({ targetLanguage: selectedProfile.profile.targetLanguage === 'ko' ? 'en' : 'ko' })}>{labels.changeTarget}</button></aside>}
      <GlossaryEditor locale={locale} entries={settings.glossary} onChange={(glossary) => setSettings({ ...settings, glossary })} />
      <ModelSelector locale={locale} choice={settings.selectedModel} models={models} onChange={(selectedModel) => setSettings({ ...settings, selectedModel })} />

      <label><input type="checkbox" checked={settings.launchAtLogin} onChange={(event) => setSettings({ ...settings, launchAtLogin: event.target.checked })} />{labels.launch}</label>
      <label>{labels.close}<select value={settings.closeBehavior} onChange={(event) => setSettings({ ...settings, closeBehavior: event.target.value as CloseBehavior })}>
        <option value="keepInTray">{labels.keepInTray}</option><option value="quit">{labels.quit}</option><option value="askEveryTime">{labels.askEveryTime}</option>
      </select></label>
      <label>{labels.quickPosition}<select value={settings.quickAccessPosition} onChange={(event) => setSettings({ ...settings, quickAccessPosition: event.target.value as QuickAccessPosition })}>
        <option value="popup">{labels.popup}</option><option value="mainWindow">{labels.mainWindow}</option>
      </select></label>
      <button type="button" onClick={() => void save()}>{labels.save}</button>
      <p role="status" aria-live="polite">{status}</p>
    </section>
  );
}
