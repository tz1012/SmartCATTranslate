import { useCallback, useEffect, useRef, useState } from 'react';
import { suspendHotkeys } from './hotkeyApi';
import type { Chord, KeyCode, Modifiers, PhysicalKey, Trigger } from './types';

const modifierCodes = new Set(['ControlLeft', 'ControlRight', 'AltLeft', 'AltRight', 'ShiftLeft', 'ShiftRight', 'MetaLeft', 'MetaRight']);

function keyCode(event: KeyboardEvent): KeyCode | null {
  if (/^Key[A-Z]$/.test(event.code)) return { kind: 'physical', value: `key${event.code.slice(3)}` as PhysicalKey };
  if (/^Digit[0-9]$/.test(event.code)) return { kind: 'physical', value: `digit${event.code.slice(5)}` as PhysicalKey };
  if (/^F([1-9]|1[0-9]|2[0-4])$/.test(event.code)) return { kind: 'physical', value: event.code.toLowerCase() as PhysicalKey };
  const physical: Record<string, PhysicalKey> = {
    Backquote: 'backquote', Backslash: 'backslash', BracketLeft: 'bracketLeft', BracketRight: 'bracketRight',
    Comma: 'comma', Equal: 'equal', Minus: 'minus', Period: 'period', Quote: 'quote', Semicolon: 'semicolon', Slash: 'slash',
    NumpadAdd: 'numpadAdd', NumpadDecimal: 'numpadDecimal', NumpadDivide: 'numpadDivide', NumpadMultiply: 'numpadMultiply',
    NumpadSubtract: 'numpadSubtract', NumpadEnter: 'numpadEnter', NumpadEqual: 'numpadEqual', PrintScreen: 'printScreen', Pause: 'pause',
  };
  if (/^Numpad[0-9]$/.test(event.code)) return { kind: 'physical', value: `numpad${event.code.slice(6)}` as PhysicalKey };
  if (physical[event.code]) return { kind: 'physical', value: physical[event.code] };
  const logical: Record<string, KeyCode> = {
    ArrowUp: { kind: 'logical', value: 'arrowUp' }, ArrowDown: { kind: 'logical', value: 'arrowDown' },
    ArrowLeft: { kind: 'logical', value: 'arrowLeft' }, ArrowRight: { kind: 'logical', value: 'arrowRight' },
    Backspace: { kind: 'logical', value: 'backspace' }, Delete: { kind: 'logical', value: 'delete' },
    End: { kind: 'logical', value: 'end' }, Enter: { kind: 'logical', value: 'enter' }, Home: { kind: 'logical', value: 'home' },
    Insert: { kind: 'logical', value: 'insert' }, PageDown: { kind: 'logical', value: 'pageDown' }, PageUp: { kind: 'logical', value: 'pageUp' }, Space: { kind: 'logical', value: 'space' },
  };
  return logical[event.code] ?? null;
}

export function chordLabel(chord: Chord) {
  const parts = [chord.modifiers.ctrl && 'Ctrl', chord.modifiers.alt && 'Alt', chord.modifiers.shift && 'Shift', chord.modifiers.meta && (navigator.platform.includes('Mac') ? '⌘' : 'Win')].filter(Boolean);
  const raw = chord.key.value.replace(/^key/, '').replace(/^digit/, '');
  parts.push(raw.replace(/^f(\d+)$/, 'F$1'));
  return parts.join('+');
}

export function triggerLabel(trigger: Trigger) {
  return (trigger.type === 'chord' ? [trigger.chord] : trigger.steps).map(chordLabel).join(', ');
}

function toTrigger(steps: Chord[]): Trigger {
  return steps.length === 1 ? { type: 'chord', chord: steps[0] } : { type: 'sequence', steps, timeoutMs: 650 };
}

export function HotkeyRecorder({ locale, value, onChange, onRecordingStart, describedBy }: { locale: 'ko' | 'en'; value: Trigger | null; onChange: (value: Trigger) => void; onRecordingStart?: () => void; describedBy?: string }) {
  const recorderRef = useRef<HTMLDivElement>(null);
  const [recording, setRecording] = useState(false);
  const [steps, setSteps] = useState<Chord[]>([]);
  const stop = useCallback(() => {
    setRecording(false);
    void suspendHotkeys(false);
  }, []);

  useEffect(() => () => { void suspendHotkeys(false); }, []);
  useEffect(() => {
    if (recording) recorderRef.current?.focus();
  }, [recording]);

  const begin = () => {
    onRecordingStart?.();
    setSteps([]);
    setRecording(true);
    void suspendHotkeys(true);
  };
  const finish = () => {
    if (steps.length) onChange(toTrigger(steps));
    stop();
  };
  const onKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (!recording) return;
    if (event.key === 'Escape') { stop(); return; }
    if (event.key === 'Tab' || event.repeat || modifierCodes.has(event.code)) return;
    const code = keyCode(event.nativeEvent);
    if (!code) return;
    event.preventDefault();
    event.stopPropagation();
    const modifiers: Modifiers = { ctrl: event.ctrlKey, alt: event.altKey, shift: event.shiftKey, meta: event.metaKey };
    if (!steps.length && !modifiers.ctrl && !modifiers.alt && !modifiers.shift && !modifiers.meta && !code.value.startsWith('f')) return;
    setSteps((current) => [...current, { modifiers, key: code }].slice(0, 4));
  };

  const shown = recording ? steps.map(chordLabel).join(', ') : value ? triggerLabel(value) : '';
  return <div ref={recorderRef} className="hotkey-recorder" tabIndex={0} role="group" aria-describedby={describedBy} aria-label={locale === 'ko' ? '단축키 녹화' : 'Record shortcut'} onKeyDown={onKeyDown}>
    <output>{shown || (locale === 'ko' ? '단축키가 지정되지 않았습니다' : 'No shortcut assigned')}</output>
    {!recording ? <button type="button" onClick={begin}>{locale === 'ko' ? '새 단축키 녹화' : 'Record shortcut'}</button> : <>
      <span role="status">{locale === 'ko' ? '원하는 키를 누르세요 (최대 4단계)' : 'Press keys (up to 4 steps)'}</span>
      <button type="button" disabled={!steps.length} onClick={finish}>{locale === 'ko' ? '완료' : 'Done'}</button>
      <button type="button" onClick={stop}>{locale === 'ko' ? '취소' : 'Cancel'}</button>
    </>}
  </div>;
}

