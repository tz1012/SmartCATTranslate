import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { AppSettings } from '../settings/SettingsView';
import {
  deleteAllHistory,
  deleteHistory,
  listHistory,
  type HistoryRecord,
} from './historyApi';

export function HistoryView({ locale }: { locale: 'ko' | 'en' }) {
  const ko = locale === 'ko';
  const [records, setRecords] = useState<HistoryRecord[]>([]);
  const [unreadableCount, setUnreadableCount] = useState(0);
  const [error, setError] = useState('');
  const [settings, setSettings] = useState<AppSettings>();
  const load = useCallback(async () => {
    setError('');
    setUnreadableCount(0);
    try {
      const page = await listHistory();
      setRecords(page.records);
      setUnreadableCount(page.unreadableCount ?? 0);
    } catch {
      setError(ko ? '기록을 열 수 없습니다.' : 'Could not open history.');
    }
  }, [ko]);

  useEffect(() => {
    load();
    void invoke<AppSettings>('get_settings').then(setSettings).catch(() => undefined);
  }, [load]);

  const retention = async (days: number) => {
    if (!settings) return;
    const saved = await invoke<AppSettings>('save_settings', {
      settings: { ...settings, historyRetentionDays: days },
    });
    setSettings(saved);
    await invoke('purge_history');
    load();
  };

  return (
    <section className="history-view">
      <header>
        <div>
          <h2>{ko ? '번역 기록' : 'Translation history'}</h2>
          <p>
            {ko
              ? '원문과 번역문은 이 기기의 운영체제 키로 암호화됩니다.'
              : 'Source and results are encrypted with this device’s OS-protected key.'}
          </p>
        </div>
        <label>
          {ko ? '보관 기간' : 'Retention'}
          <select
            value={settings?.historyRetentionDays ?? 30}
            disabled={!settings}
            onChange={(event) => void retention(Number(event.target.value))}
          >
            <option value="7">7</option>
            <option value="30">30</option>
            <option value="90">90</option>
            <option value="365">365</option>
          </select>
        </label>
        <button
          type="button"
          disabled={!records.length}
          onClick={() => void deleteAllHistory().then(() => setRecords([]))}
        >
          {ko ? '전체 삭제' : 'Delete all'}
        </button>
      </header>
      {error && <div className="history-load-error" role="alert">
        <p>{error}</p>
        <button type="button" onClick={() => void load()}>{ko ? '다시 시도' : 'Retry'}</button>
      </div>}
      {unreadableCount > 0 && <p className="history-unreadable-warning" role="status">
        {ko
          ? `손상되어 열 수 없는 기록 ${unreadableCount}건을 건너뛰었습니다.`
          : `Skipped ${unreadableCount} damaged history record${unreadableCount === 1 ? '' : 's'} that could not be opened.`}
      </p>}
      {!records.length && !error && <p>{ko ? '저장된 기록이 없습니다.' : 'No saved history.'}</p>}
      <ol>
        {records.map((record) => (
          <li key={record.id}>
            <header>
              <strong>
                {record.displayName ??
                  (record.kind === 'capture'
                    ? ko
                      ? '이미지 번역'
                      : 'Image translation'
                    : ko
                      ? '텍스트 번역'
                      : 'Text translation')}
              </strong>
              <time>{new Date(record.createdAt).toLocaleString()}</time>
            </header>
            {record.source && <p>{record.source}</p>}
            {record.result && (
              <textarea
                aria-label={ko ? '번역문' : 'Translation'}
                readOnly
                value={record.result}
              />
            )}
            <div>
              <button
                type="button"
                disabled={!record.result}
                onClick={() => void navigator.clipboard.writeText(record.result)}
              >
                {ko ? '복사' : 'Copy'}
              </button>
              <button
                type="button"
                onClick={() =>
                  void deleteHistory(record.id).then(() =>
                    setRecords((items) => items.filter((item) => item.id !== record.id)),
                  )
                }
              >
                {ko ? '삭제' : 'Delete'}
              </button>
            </div>
          </li>
        ))}
      </ol>
    </section>
  );
}
