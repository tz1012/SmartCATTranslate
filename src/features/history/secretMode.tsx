import { useEffect, useState } from 'react';

const KEY = 'smartcat.secretMode';
const EVENT = 'smartcat-secret-mode';

export function readSecretMode() {
  try {
    return localStorage.getItem(KEY) === 'true';
  } catch {
    return false;
  }
}

export function setSecretMode(value: boolean) {
  try {
    localStorage.setItem(KEY, String(value));
  } catch {
    // Secret mode still works when preference persistence is unavailable.
  }
  window.dispatchEvent(new CustomEvent(EVENT, { detail: value }));
}

export function useSecretMode() {
  const [value, setValue] = useState(readSecretMode);
  useEffect(() => {
    const sync = (event: Event) =>
      setValue(event instanceof CustomEvent ? Boolean(event.detail) : readSecretMode());
    window.addEventListener(EVENT, sync);
    window.addEventListener('storage', sync);
    return () => {
      window.removeEventListener(EVENT, sync);
      window.removeEventListener('storage', sync);
    };
  }, []);
  return [value, (next: boolean) => setSecretMode(next)] as const;
}

export function SecretModeSwitch({
  locale,
  value,
  onChange,
}: {
  locale: 'ko' | 'en';
  value: boolean;
  onChange: (value: boolean) => void;
}) {
  const ko = locale === 'ko';
  return (
    <label className="secret-mode-switch">
      <input
        type="checkbox"
        checked={value}
        onChange={(event) => onChange(event.target.checked)}
      />
      <strong>{ko ? '시크릿 번역' : 'Secret translation'}</strong>
      <span>
        {value
          ? ko
            ? '디스크에 저장하지 않음 · 앱 종료 시 복구 정보 삭제'
            : 'Not saved to disk · recovery is cleared when the app closes'
          : ko
            ? '로컬 암호화 기록 사용'
            : 'Encrypted local history enabled'}
      </span>
    </label>
  );
}
