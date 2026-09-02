import { useEffect, useRef, useState } from 'react';
import { createUuidV4 } from '../settings/uuid';
import type { HotkeyBinding, Trigger } from './types';
import { analyzeHotkey, listBlockedApps, listHotkeys, saveBlockedApps, saveHotkey, type BlockedApp, type ConflictReport } from './hotkeyApi';
import { HotkeyRecorder, triggerLabel } from './HotkeyRecorder';
import { detectDesktopPlatform } from '../../lib/platform';

const emptyReport: ConflictReport = { level: 'none', causes: [], alternatives: [], canForce: false };

export function HotkeySettings({ locale, defaultProfileId }: { locale: 'ko' | 'en'; defaultProfileId: string }) {
  const ko = locale === 'ko';
  const platform = detectDesktopPlatform();
  const [bindings, setBindings] = useState<HotkeyBinding[]>([]);
  const [trigger, setTrigger] = useState<Trigger | null>(null);
  const [report, setReport] = useState<ConflictReport>(emptyReport);
  const [analyzing, setAnalyzing] = useState(false);
  const [forceWarning, setForceWarning] = useState(false);
  const [status, setStatus] = useState('');
  const [blockedApps, setBlockedApps] = useState<BlockedApp[]>([]);
  const [appName, setAppName] = useState('');
  const [identity, setIdentity] = useState('');
  const analysisVersion = useRef(0);

  useEffect(() => { void Promise.all([listHotkeys(), listBlockedApps()]).then(([hotkeys, blocked]) => { setBindings(hotkeys); setBlockedApps(blocked); }).catch(() => setStatus(ko ? '단축키 설정을 불러오지 못했습니다.' : 'Could not load shortcut settings.')); }, [ko]);

  const inspect = async (next: Trigger) => {
    const version = ++analysisVersion.current;
    setTrigger(next); setForceWarning(false); setAnalyzing(true); setStatus('');
    try {
      const nextReport = await analyzeHotkey(next);
      if (version === analysisVersion.current) setReport(nextReport);
    } catch {
      if (version === analysisVersion.current) {
        setReport(emptyReport);
        setStatus(ko ? '충돌을 확인하지 못했습니다.' : 'Could not check conflicts.');
      }
    } finally {
      if (version === analysisVersion.current) setAnalyzing(false);
    }
  };
  const persist = async (force: boolean) => {
    if (!trigger) return;
    try {
      const matching = bindings.find((binding) => binding.action === 'translateSelection');
      setBindings(await saveHotkey({ id: matching?.id ?? createUuidV4(), trigger, action: 'translateSelection', profileId: matching?.profileId ?? defaultProfileId, force }));
      setForceWarning(false); setStatus(ko ? '단축키를 저장했습니다.' : 'Shortcut saved.');
    } catch { setStatus(ko ? '단축키를 저장하지 못했습니다.' : 'Could not save shortcut.'); }
  };
  const conflict = report.level !== 'none';

  const addBlocked = async () => {
    const cleanIdentity = identity.trim();
    if (!cleanIdentity) return;
    const entry: BlockedApp = { platform, executable: platform === 'windows' ? cleanIdentity : null, bundleId: platform === 'macos' ? cleanIdentity : null, catalogName: appName.trim() || cleanIdentity };
    try { setBlockedApps(await saveBlockedApps([...blockedApps, entry])); setAppName(''); setIdentity(''); } catch { setStatus(ko ? '차단 프로그램을 추가하지 못했습니다.' : 'Could not add blocked app.'); }
  };
  const removeBlocked = async (index: number) => {
    try { setBlockedApps(await saveBlockedApps(blockedApps.filter((_, itemIndex) => itemIndex !== index))); } catch { setStatus(ko ? '차단 프로그램을 삭제하지 못했습니다.' : 'Could not remove blocked app.'); }
  };

  return <fieldset className="hotkey-settings">
    <legend>{ko ? '빠른 번역 단축키' : 'Quick translation shortcut'}</legend>
    <p>{ko ? '다른 프로그램에서 선택한 문장을 번역할 단축키를 정합니다.' : 'Choose a shortcut to translate selected text in other apps.'}</p>
    <HotkeyRecorder
      locale={locale}
      value={trigger ?? bindings.find((binding) => binding.action === 'translateSelection')?.trigger ?? null}
      onChange={(next) => void inspect(next)}
      onRecordingStart={() => {
        analysisVersion.current += 1;
        setTrigger(null);
        setReport(emptyReport);
        setAnalyzing(false);
        setForceWarning(false);
        setStatus('');
      }}
      describedBy="hotkey-conflict-help"
    />
    <div id="hotkey-conflict-help" aria-live="polite">
      {analyzing && <p role="status">{ko ? '다른 프로그램과 겹치는지 확인 중…' : 'Checking for conflicts…'}</p>}
      {report.causes.map((cause, index) => <article className={`conflict-cause ${cause.severity}`} key={`${cause.description}-${index}`}>
        <strong>{cause.application ?? (ko ? '운영체제 또는 다른 프로그램' : 'Operating system or another app')}</strong>
        {cause.feature && <span>{ko ? '기존 기능: ' : 'Existing action: '}{cause.feature}</span>}
        <p>{cause.description}</p>
        {cause.severity === 'advisory' && <small>{ko ? '연속 단축키의 앞부분만 같아 저장할 수 있지만, 입력할 때 기존 기능도 실행될 수 있습니다.' : 'Only the prefix overlaps. You can save it, but the existing action may also run.'}</small>}
      </article>)}
    </div>
    {report.alternatives.length > 0 && <div className="hotkey-alternatives"><strong>{ko ? '사용 가능한 대체 단축키' : 'Available alternatives'}</strong>{report.alternatives.map((alternative) => <button type="button" key={triggerLabel(alternative)} onClick={() => void inspect(alternative)}>{triggerLabel(alternative)}</button>)}</div>}
    <div className="hotkey-actions">
      <button type="button" className="primary-action" disabled={!trigger || analyzing || conflict} onClick={() => void persist(false)}>{ko ? '저장' : 'Save'}</button>
      {conflict && report.canForce && !forceWarning && <button type="button" className="danger-action" onClick={() => setForceWarning(true)}>{ko ? '경고 후 강제로 지정' : 'Force after warning'}</button>}
    </div>
    {forceWarning && <aside className="force-warning" role="alert"><p>{ko ? '기존 프로그램 기능이 함께 실행되거나 단축키가 작동하지 않을 수 있습니다.' : 'The existing app action may also run, or this shortcut may not work.'}</p><button type="button" className="danger-action" onClick={() => void persist(true)}>{ko ? '위험을 이해했고 강제 저장' : 'I understand, save anyway'}</button><button type="button" onClick={() => setForceWarning(false)}>{ko ? '취소' : 'Cancel'}</button></aside>}
    {conflict && !report.canForce && <p role="alert">{ko ? '운영체제 예약 또는 권한 문제로 강제 지정할 수 없습니다. 대체 단축키를 선택하세요.' : 'This cannot be forced because it is reserved or permission is missing. Choose an alternative.'}</p>}

    <section className="blocklist-editor" aria-labelledby="blocklist-title">
      <h3 id="blocklist-title">{ko ? '사용하지 않을 프로그램' : 'Blocked apps'}</h3>
      <p>{ko ? '이 프로그램에서는 선택문을 읽거나 번역 팝업을 열지 않습니다.' : 'SmartCAT will not read selections or open a popup in these apps.'}</p>
      {blockedApps.map((app, index) => <div className="blocked-app" key={`${app.platform}-${app.executable}-${app.bundleId}`}><span><strong>{app.catalogName}</strong><small>{app.executable ?? app.bundleId}</small></span><button type="button" onClick={() => void removeBlocked(index)}>{ko ? '삭제' : 'Remove'}</button></div>)}
      <div className="blocklist-add"><label>{ko ? '프로그램 이름' : 'App name'}<input value={appName} maxLength={128} onChange={(event) => setAppName(event.target.value)} /></label><label>{platform === 'macos' ? 'Bundle ID' : (ko ? '실행 파일 이름' : 'Executable name')}<input value={identity} maxLength={128} placeholder={platform === 'macos' ? 'com.example.app' : 'example.exe'} onChange={(event) => setIdentity(event.target.value)} /></label><button type="button" disabled={!identity.trim()} onClick={() => void addBlocked()}>{ko ? '추가' : 'Add'}</button></div>
    </section>
    {platform === 'macos'
      ? <aside className="permission-note"><strong>{ko ? '접근성 및 키보드 권한' : 'Accessibility and keyboard permissions'}</strong><p>{ko ? 'macOS에서 연속 단축키를 사용하려면 시스템 설정의 개인정보 보호 및 보안에서 SmartCAT Translate의 접근성 권한을 켜야 할 수 있습니다.' : 'On macOS, sequences may require Accessibility permission in System Settings and Privacy & Security.'}</p></aside>
      : <aside className="permission-note"><strong>{ko ? 'Windows 단축키 권한' : 'Windows shortcut permissions'}</strong><p>{ko ? 'Windows에서 단축키가 작동하지 않으면 보안 프로그램의 키보드 입력 차단 설정을 확인하세요.' : 'On Windows, if a global shortcut is blocked, check the keyboard-input settings in your security software.'}</p></aside>}
    <p role="status" aria-live="polite">{status}</p>
  </fieldset>;
}

