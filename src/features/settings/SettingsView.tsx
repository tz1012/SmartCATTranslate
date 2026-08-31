import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { Field, Quality, Tone, TranslationProfile } from '../../lib/types';
export type { Field } from '../../lib/types';
import { onAccountStateChanged } from '../account/accountApi';
import { GlossaryEditor } from './GlossaryEditor';
import { languageLabel, SUPPORTED_LANGUAGES } from './languages';
import { ModelSelector, modelChoiceValue } from './ModelSelector';
import { UpdatePanel } from './UpdatePanel';
import { createUuidV4 } from './uuid';
import { HotkeySettings } from '../hotkeys/HotkeySettings';
import { SettingsCategories, type SettingsCategory } from './SettingsCategories';

export type AppLocale = 'ko' | 'en';
export type Theme = 'system' | 'light' | 'dark';
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
    modelListStatus: '모델 목록 상태', modelListError: '모델 목록을 불러올 수 없습니다. 저장된 선택을 유지합니다.', signedOutModels: 'ChatGPT 계정 연결 후 모델 목록을 확인할 수 있습니다.',
    retryModels: '다시 시도', installedOnly: '설치된 앱에서만 사용할 수 있습니다', lifecycleError: '운영체제 설정을 변경하지 못했습니다', pauseHotkeys: '단축키 일시 중지',
    rewritePrompt: '문장을 개선할까요?', rewrite: '문장 개선', changeTarget: '대상 언어 변경', defaultProfile: '기본 프로필', newProfile: '새 프로필',
    retention: '번역 기록 보관 기간', retentionUnit: '일',
  },
  en: {
    title: 'Settings', loading: 'Loading settings', loadError: 'Could not load settings', profiles: 'Translation profiles',
    addProfile: 'Add profile', deleteProfile: 'Delete profile', profileName: 'Profile name', locale: 'Interface language', theme: 'Theme',
    source: 'Source language', target: 'Target language', quality: 'Quality', tone: 'Tone', field: 'Field', auto: 'Detect automatically',
    system: 'System', light: 'Light', dark: 'Dark', fast: 'Fast', balanced: 'Balanced', precise: 'Precise', natural: 'Natural',
    literal: 'Literal', formal: 'Formal', casual: 'Casual', general: 'General', technical: 'Technical', legal: 'Legal', medical: 'Medical', business: 'Business',
    launch: 'Launch at login', close: 'Close behavior', keepInTray: 'Keep in tray', quit: 'Quit app', askEveryTime: 'Ask every time',
    quickPosition: 'Quick translation position', popup: 'Small popup', mainWindow: 'Main window', save: 'Save settings', saved: 'Saved', saveError: 'Could not save settings',
    modelListStatus: 'Model list status', modelListError: 'Could not load the model list. The saved selection is preserved.', signedOutModels: 'Connect a ChatGPT account to view available models.',
    retryModels: 'Retry', installedOnly: 'Available in the installed app only', lifecycleError: 'Could not change the operating system setting', pauseHotkeys: 'Pause hotkeys',
    rewritePrompt: 'Improve the sentence?', rewrite: 'Improve writing', changeTarget: 'Change target language', defaultProfile: 'Default profile', newProfile: 'New profile',
    retention: 'Keep translation history', retentionUnit: 'days',
  },
} as const;

function newId() {
  return createUuidV4();
}

function displayProfileName(profile: SavedProfile, settings: AppSettings, locale: AppLocale) {
  return locale === 'en' && profile.id === settings.defaultProfileId && profile.name === '기본 프로필'
    ? copy.en.defaultProfile
    : profile.name;
}

export function SettingsView({
  locale: localeOverride,
  detectedSourceLanguage,
  onRewrite,
  onPreferencesLoaded,
  onPreferencesSaved,
  initialCategory = 'translation',
}: {
  locale?: AppLocale;
  detectedSourceLanguage?: string;
  onRewrite?: () => void;
  onPreferencesLoaded?: (locale: AppLocale, theme: Theme) => void;
  onPreferencesSaved?: (locale: AppLocale, theme: Theme) => void;
  initialCategory?: SettingsCategory;
}) {
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [models, setModels] = useState<AvailableModel[]>([]);
  const [modelCatalogStatus, setModelCatalogStatus] = useState<'loading' | 'available' | 'unavailable' | 'signedOut'>('loading');
  const [selectedProfileId, setSelectedProfileId] = useState('');
  const [status, setStatus] = useState('');
  const [loadFailed, setLoadFailed] = useState(false);
  const [launchAtLoginAvailable, setLaunchAtLoginAvailable] = useState(true);
  const [hotkeysPaused, setHotkeysPaused] = useState(false);
  const [category, setCategory] = useState<SettingsCategory>(initialCategory);
  const settingsRevision = useRef(0);
  const saveGeneration = useRef(0);
  const mounted = useRef(false);
  const modelRefreshGeneration = useRef(0);

  useEffect(() => setCategory(initialCategory), [initialCategory]);

  const refreshModels = useCallback(async () => {
    if (!mounted.current) return;
    const generation = ++modelRefreshGeneration.current;
    setModelCatalogStatus('loading');
    try {
      const available = await invoke<AvailableModel[]>('list_available_models');
      if (!mounted.current || generation !== modelRefreshGeneration.current) return;
      setModels(available);
      setModelCatalogStatus('available');
    } catch (error) {
      if (!mounted.current || generation !== modelRefreshGeneration.current) return;
      setModels([]);
      setModelCatalogStatus(error === 'model_catalog_signed_out' ? 'signedOut' : 'unavailable');
    }
  }, []);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    mounted.current = true;
    void invoke<AppSettings>('get_settings').then((loadedSettings) => {
      if (disposed) return;
      setSettings(loadedSettings);
      setSelectedProfileId(loadedSettings.defaultProfileId);
      onPreferencesLoaded?.(loadedSettings.locale, loadedSettings.theme);
      void invoke<{ launchAtLoginAvailable: boolean; launchAtLoginEnabled: boolean; hotkeysPaused: boolean }>('get_lifecycle_status')
        .then((lifecycle) => {
          if (disposed) return;
          setLaunchAtLoginAvailable(lifecycle.launchAtLoginAvailable);
          setHotkeysPaused(lifecycle.hotkeysPaused);
          setSettings((current) => current ? { ...current, launchAtLogin: lifecycle.launchAtLoginEnabled } : current);
        }).catch(() => !disposed && setLaunchAtLoginAvailable(false));
    }).catch(() => !disposed && setLoadFailed(true));
    let stopPause: (() => void) | undefined;
    void listen<boolean>('hotkeys-paused', (event) => setHotkeysPaused(event.payload)).then((stop) => {
      if (disposed) stop(); else stopPause = stop;
    });
    try {
      void onAccountStateChanged(() => {
        void refreshModels();
      }).then((stop) => {
        if (disposed) {
          stop();
        } else {
          unlisten = stop;
          void refreshModels();
        }
      }).catch(() => {
        if (!disposed) void refreshModels();
      });
    } catch {
      void refreshModels();
    }
    return () => {
      disposed = true;
      mounted.current = false;
      modelRefreshGeneration.current += 1;
      unlisten?.();
      stopPause?.();
    };
  }, [onPreferencesLoaded, refreshModels]);

  const locale = localeOverride ?? settings?.locale ?? 'ko';
  const labels = copy[locale];
  const selectedProfile = useMemo(() => settings?.profiles.find((profile) => profile.id === selectedProfileId)
    ?? settings?.profiles.find((profile) => profile.id === settings.defaultProfileId), [settings, selectedProfileId]);

  if (loadFailed) return <section aria-label={labels.title}><p role="alert">{labels.loadError}</p></section>;
  if (!settings || !selectedProfile) return <section aria-label={labels.title}><p role="status">{labels.loading}</p></section>;

  const editSettings = (updater: (current: AppSettings) => AppSettings) => {
    settingsRevision.current += 1;
    setSettings((current) => current ? updater(current) : current);
  };

  const updateProfile = (updater: (profile: SavedProfile) => SavedProfile) => {
    editSettings((current) => ({
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
    editSettings((current) => ({ ...current, profiles: [...current.profiles, profile] }));
    setSelectedProfileId(profile.id);
  };
  const deleteProfile = () => {
    if (selectedProfile.id === settings.defaultProfileId) return;
    editSettings((current) => ({ ...current, profiles: current.profiles.filter((profile) => profile.id !== selectedProfile.id) }));
    setSelectedProfileId(settings.defaultProfileId);
  };

  const save = async () => {
    const savedModel = modelCatalogStatus === 'available' && modelChoiceValue(settings.selectedModel, models) === 'automatic'
      ? { type: 'automatic' } as const
      : settings.selectedModel;
    const candidate = { ...settings, selectedModel: savedModel };
    const revision = settingsRevision.current;
    const generation = ++saveGeneration.current;
    try {
      const saved = await invoke<AppSettings>('save_settings', { settings: candidate });
      if (generation === saveGeneration.current && revision === settingsRevision.current) {
        setSettings(saved);
        setStatus(labels.saved);
        onPreferencesSaved?.(saved.locale, saved.theme);
      }
    } catch {
      if (generation === saveGeneration.current) setStatus(labels.saveError);
    }
  };

  const updateLifecycle = async (
    command: 'set_launch_at_login' | 'set_close_behavior' | 'set_quick_access_position',
    args: Record<string, unknown>,
  ) => {
    try {
      const saved = await invoke<AppSettings>(command, args);
      setSettings(saved);
      settingsRevision.current += 1;
      setStatus(labels.saved);
    } catch {
      setStatus(command === 'set_launch_at_login' && !launchAtLoginAvailable ? labels.installedOnly : labels.lifecycleError);
    }
  };

  const effectiveSourceLanguage = selectedProfile.profile.sourceLanguage ?? detectedSourceLanguage;
  const rewriteSuggested = Boolean(effectiveSourceLanguage
    && effectiveSourceLanguage.toLowerCase() === selectedProfile.profile.targetLanguage.toLowerCase());

  return (
    <section className="settings-view" aria-labelledby="settings-title">
      <h2 id="settings-title">{labels.title}</h2>
      <div className="settings-shell">
        <SettingsCategories locale={locale} value={category} onChange={setCategory} />
        <div id={`settings-panel-${category}`} className="settings-content" role="tabpanel" aria-labelledby={`settings-tab-${category}`}>
          {category === 'general' && <div className="settings-form-grid">
            <label>{labels.locale}<select value={settings.locale} onChange={(event) => editSettings((current) => ({ ...current, locale: event.target.value as AppLocale }))}>
              <option value="ko">{locale === 'ko' ? '한국어' : 'Korean'}</option><option value="en">English</option>
            </select></label>
            <label>{labels.theme}<select value={settings.theme} onChange={(event) => editSettings((current) => ({ ...current, theme: event.target.value as Theme }))}>
              <option value="system">{labels.system}</option><option value="light">{labels.light}</option><option value="dark">{labels.dark}</option>
            </select></label>
            <label className="settings-check"><input type="checkbox" checked={settings.launchAtLogin} disabled={!launchAtLoginAvailable} onChange={(event) => void updateLifecycle('set_launch_at_login', { enabled: event.target.checked })} />{labels.launch}</label>
            {!launchAtLoginAvailable && <p className="settings-help">{labels.installedOnly}</p>}
            <label>{labels.quickPosition}<select value={settings.quickAccessPosition} onChange={(event) => void updateLifecycle('set_quick_access_position', { quickAccessPosition: event.target.value as QuickAccessPosition })}>
              <option value="popup">{labels.popup}</option><option value="mainWindow">{labels.mainWindow}</option>
            </select></label>
          </div>}

          {category === 'translation' && <>
            <fieldset className="profile-settings-grid">
              <legend>{labels.profiles}</legend>
              <div className="profile-picker">
                <select aria-label={labels.profiles} value={selectedProfile.id} onChange={(event) => setSelectedProfileId(event.target.value)}>
                  {settings.profiles.map((profile) => <option key={profile.id} value={profile.id}>{displayProfileName(profile, settings, locale)}</option>)}
                </select>
                <button type="button" onClick={addProfile}>{labels.addProfile}</button>
                <button type="button" disabled={selectedProfile.id === settings.defaultProfileId} onClick={deleteProfile}>{labels.deleteProfile}</button>
              </div>
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
            <GlossaryEditor locale={locale} entries={settings.glossary} onChange={(glossary) => editSettings((current) => ({ ...current, glossary }))} />
            <ModelSelector locale={locale} choice={settings.selectedModel} models={models} catalogStatus={modelCatalogStatus} onChange={(selectedModel) => editSettings((current) => ({ ...current, selectedModel }))} />
            {modelCatalogStatus === 'unavailable' && <div><p role="status" aria-label={labels.modelListStatus}>{labels.modelListError}</p><button type="button" onClick={() => void refreshModels()}>{labels.retryModels}</button></div>}
            {modelCatalogStatus === 'signedOut' && <div><p role="status" aria-label={labels.modelListStatus}>{labels.signedOutModels}</p><button type="button" onClick={() => void refreshModels()}>{labels.retryModels}</button></div>}
          </>}

          {category === 'shortcuts' && <>
            <HotkeySettings locale={locale} defaultProfileId={settings.defaultProfileId} />
            <label className="settings-check"><input type="checkbox" checked={hotkeysPaused} onChange={(event) => {
              const paused = event.target.checked;
              setHotkeysPaused(paused);
              void invoke('set_hotkeys_paused', { paused }).catch(() => setHotkeysPaused(!paused));
            }} />{labels.pauseHotkeys}</label>
          </>}

          {category === 'privacy' && <div className="settings-form-grid">
            <label>{labels.retention}<span className="settings-inline-input"><input type="number" min="1" max="365" value={settings.historyRetentionDays} onChange={(event) => editSettings((current) => ({ ...current, historyRetentionDays: Math.min(365, Math.max(1, Number(event.target.value) || 1)) }))} /><span>{labels.retentionUnit}</span></span></label>
            <label>{labels.close}<select value={settings.closeBehavior} onChange={(event) => void updateLifecycle('set_close_behavior', { closeBehavior: event.target.value as CloseBehavior })}>
              <option value="keepInTray">{labels.keepInTray}</option><option value="quit">{labels.quit}</option><option value="askEveryTime">{labels.askEveryTime}</option>
            </select></label>
          </div>}

          {category === 'updates' && <UpdatePanel locale={locale} />}
        </div>
      </div>
      <div className="settings-footer"><button type="button" onClick={() => void save()}>{labels.save}</button><p role="status" aria-live="polite">{status}</p></div>
    </section>
  );
}
