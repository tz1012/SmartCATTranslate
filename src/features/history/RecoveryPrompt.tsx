import { useEffect, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import {
  deleteRecoveryJob,
  listRecoverableJobs,
  prepareDocumentRecovery,
  type PreparedDocumentRecovery,
  type RecoverableJob,
} from './historyApi';

function recoveryAge(createdAt: string, ko: boolean) {
  const elapsedMinutes = Math.max(0, Math.floor((Date.now() - Date.parse(createdAt)) / 60_000));
  if (elapsedMinutes < 60) return ko ? `${elapsedMinutes}분 전` : `${elapsedMinutes}m ago`;
  const hours = Math.floor(elapsedMinutes / 60);
  if (hours < 24) return ko ? `${hours}시간 전` : `${hours}h ago`;
  const days = Math.floor(hours / 24);
  return ko ? `${days}일 전` : `${days}d ago`;
}

export function RecoveryPrompt({
  locale,
  onContinue,
}: {
  locale: 'ko' | 'en';
  onContinue: (value: PreparedDocumentRecovery) => void;
}) {
  const ko = locale === 'ko';
  const [jobs, setJobs] = useState<RecoverableJob[]>([]);

  useEffect(() => {
    let disposed = false;
    let stop: (() => void) | undefined;
    const refresh = () => {
      void listRecoverableJobs()
        .then((values) => {
          if (!disposed) setJobs(values);
        })
        .catch(() => undefined);
    };
    refresh();
    void listen('recovery-updated', refresh).then((unlisten) => {
      if (disposed) unlisten();
      else stop = unlisten;
    });
    return () => {
      disposed = true;
      stop?.();
    };
  }, []);

  if (!jobs.length) return null;
  const job = jobs[0];
  return (
    <aside className="recovery-prompt" role="dialog" aria-label={ko ? '중단 작업 복구' : 'Recover interrupted work'}>
      <h2>{ko ? '중단된 문서 번역이 있습니다' : 'An interrupted document translation is available'}</h2>
      <p>
        <strong>{job.displayName}</strong> · {job.stage} · {job.completed}/{job.total} · {recoveryAge(job.createdAt, ko)}
      </p>
      {job.secret && (
        <p role="status">
          {ko
            ? '시크릿 복구 정보는 메모리에만 있으며 앱을 종료하거나 다시 시작하면 사라집니다.'
            : 'Secret recovery stays in memory only and disappears when the app closes or restarts.'}
        </p>
      )}
      {!job.canResume && (
        <p role="alert">
          {ko ? '원본 또는 번역 설정이 변경되어 새 작업만 시작할 수 있습니다.' : 'The source or translation settings changed. Start a new job.'}
        </p>
      )}
      <div>
        <button type="button" disabled={!job.canResume} onClick={() => void prepareDocumentRecovery(job.recordId).then(onContinue)}>
          {ko ? '계속' : 'Continue'}
        </button>
        <button type="button" onClick={() => void deleteRecoveryJob(job.recordId).then(() => setJobs((values) => values.slice(1)))}>
          {ko ? '삭제' : 'Delete'}
        </button>
        <button type="button" onClick={() => setJobs((values) => values.slice(1))}>
          {ko ? '나중에' : 'Later'}
        </button>
      </div>
    </aside>
  );
}
