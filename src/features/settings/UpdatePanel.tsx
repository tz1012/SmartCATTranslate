import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

type UpdateInfo = { available: boolean; version?: string; releaseNotes?: string; publishedAt?: string; sizeBytes?: number; consentToken?: string; manualOnly?: boolean; releaseUrl?: string };
type Progress = { version: string; downloadedBytes: number; totalBytes?: number };
type Prepared = { installToken: string; sizeBytes: number };
type Recovery = { previousVersion: string; previousInstallerUrl: string; message: string };

const messages: Record<string, string> = {
  updater_not_configured: '이 빌드에는 업데이트 채널이 구성되지 않았습니다.',
  update_network_error: '업데이트 서버에 연결하지 못했습니다. 네트워크를 확인하고 다시 시도하세요.',
  update_signature_invalid: '업데이트 서명을 확인할 수 없어 설치를 중단했습니다.',
  update_consent_expired: '15분 승인 시간이 지나 다시 확인해야 합니다.',
  update_consent_invalid: '이 승인은 이미 사용되었거나 유효하지 않습니다.',
  update_version_mismatch: '표시한 버전과 설치할 버전이 달라 중단했습니다.',
  update_install_failed: '업데이트 설치에 실패했습니다. 현재 버전은 그대로 유지됩니다.',
  update_restart_consent_invalid: '재시작 승인이 유효하지 않습니다. 설치를 다시 준비하세요.',
  update_restart_consent_expired: '재시작 승인 시간이 지나 설치를 다시 확인해야 합니다.',
  update_restart_consent_mismatch: '재시작 승인과 설치할 버전이 달라 중단했습니다.',
};

export function UpdatePanel({ locale = 'ko' }: { locale?: 'ko' | 'en' }) {
  const [update, setUpdate] = useState<UpdateInfo>();
  const [prepared, setPrepared] = useState<Prepared>();
  const [progress, setProgress] = useState<Progress>();
  const [recovery, setRecovery] = useState<Recovery>();
  const [status, setStatus] = useState('');
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let stop: (() => void) | undefined;
    let disposed = false;
    void listen<Progress>('update-progress', (event) => setProgress(event.payload)).then((unlisten) => {
      if (disposed) unlisten(); else stop = unlisten;
    });
    void invoke<Recovery | null>('get_update_recovery_instructions').then((value) => { if (value) setRecovery(value); }).catch(() => undefined);
    return () => { disposed = true; stop?.(); };
  }, []);

  const fail = (error: unknown) => {
    const code = typeof error === 'string' ? error : '';
    setStatus(messages[code] ?? (locale === 'ko' ? '업데이트 작업을 완료하지 못했습니다.' : 'The update operation could not be completed.'));
  };
  const check = async () => {
    setBusy(true); setStatus(''); setPrepared(undefined); setProgress(undefined);
    try {
      const result = await invoke<UpdateInfo>('check_for_update');
      setUpdate(result);
      if (!result.available) setStatus(locale === 'ko' ? '최신 버전입니다.' : 'You are up to date.');
    } catch (error) { fail(error); } finally { setBusy(false); }
  };
  const download = async () => {
    if (!update?.version || !update.consentToken || update.manualOnly) return;
    setBusy(true); setStatus(locale === 'ko' ? '다운로드 및 서명 확인 중…' : 'Downloading and verifying signature…');
    try {
      const value = await invoke<Prepared>('prepare_update', { version: update.version, consentToken: update.consentToken });
      setPrepared(value);
      setStatus(locale === 'ko' ? '서명이 확인되었습니다. 설치하려면 재시작을 승인하세요.' : 'Signature verified. Approve restart to install.');
    } catch (error) { fail(error); } finally { setBusy(false); }
  };
  const install = async () => {
    if (!update?.version || !prepared) return;
    const confirmed = window.confirm(locale === 'ko'
      ? `${update.version} 설치를 위해 앱을 재시작할까요? 저장하지 않은 작업을 먼저 저장하세요.`
      : `Restart the app to install ${update.version}? Save unfinished work first.`);
    if (!confirmed) { setStatus(locale === 'ko' ? '설치를 취소했습니다. 자동으로 재시작하지 않습니다.' : 'Installation cancelled. The app will not restart automatically.'); return; }
    setBusy(true);
    try {
      const consent = await invoke<{ restartConsentToken: string }>('authorize_update_restart', {
        version: update.version,
        installToken: prepared.installToken,
      });
      await invoke('install_update', {
        version: update.version,
        installToken: prepared.installToken,
        restartConsentToken: consent.restartConsentToken,
      });
    } catch (error) { fail(error); setBusy(false); }
  };

  const percent = progress?.totalBytes ? Math.min(100, Math.round((progress.downloadedBytes / progress.totalBytes) * 100)) : undefined;
  const size = update?.sizeBytes ?? prepared?.sizeBytes;
  return <section className="update-panel" aria-labelledby="update-title">
    <h3 id="update-title">{locale === 'ko' ? '앱 업데이트' : 'App update'}</h3>
    <p>{locale === 'ko' ? '자동 다운로드·설치·재시작은 하지 않습니다.' : 'The app never downloads, installs, or restarts in the background.'}</p>
    <button type="button" disabled={busy} onClick={() => void check()}>{locale === 'ko' ? '업데이트 확인' : 'Check for updates'}</button>
    {update?.available && update.version && <article>
      <h4>{locale === 'ko' ? `새 버전 ${update.version}` : `New version ${update.version}`}</h4>
      <dl><div><dt>{locale === 'ko' ? '게시일' : 'Published'}</dt><dd>{update.publishedAt ?? '—'}</dd></div><div><dt>{locale === 'ko' ? '크기' : 'Size'}</dt><dd>{size ? `${(size / 1_048_576).toFixed(1)} MB` : (locale === 'ko' ? '서버에서 제공하지 않음' : 'Not provided')}</dd></div></dl>
      <h5>{locale === 'ko' ? '변경 내용' : 'Release notes'}</h5>
      <p className="update-notes">{update.releaseNotes || (locale === 'ko' ? '변경 내용이 제공되지 않았습니다.' : 'No release notes provided.')}</p>
      {!prepared && <div className="update-actions">
        {update.manualOnly && update.releaseUrl
          ? <button type="button" disabled={busy} onClick={() => void invoke('open_update_release', { url: update.releaseUrl }).catch(fail)}>{locale === 'ko' ? '릴리스 페이지 열기' : 'Open release page'}</button>
          : <button type="button" disabled={busy} onClick={() => void download()}>{locale === 'ko' ? '다운로드 및 설치 준비' : 'Download and prepare'}</button>}
        <button type="button" disabled={busy} onClick={() => { setUpdate(undefined); setStatus(locale === 'ko' ? '나중에 다시 확인할 수 있습니다.' : 'You can check again later.'); }}>{locale === 'ko' ? '나중에' : 'Later'}</button>
      </div>}
      {progress && <progress aria-label={locale === 'ko' ? '업데이트 다운로드' : 'Update download'} value={percent ?? progress.downloadedBytes} max={percent === undefined ? undefined : 100}>{percent}%</progress>}
      {prepared && <button type="button" disabled={busy} onClick={() => void install()}>{locale === 'ko' ? '설치하고 재시작' : 'Install and restart'}</button>}
    </article>}
    <p role="status" aria-live="polite">{status}</p>
    {recovery && <aside><h4>{locale === 'ko' ? '이전 버전 복구' : 'Previous version recovery'}</h4><p>{recovery.message}</p><button type="button" onClick={() => void invoke('open_previous_installer').catch(fail)}>{locale === 'ko' ? `이전 설치 관리자 열기 (${recovery.previousVersion})` : `Open previous installer (${recovery.previousVersion})`}</button></aside>}
  </section>;
}
