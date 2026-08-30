export interface Modifiers {
  ctrl: boolean;
  alt: boolean;
  shift: boolean;
  meta: boolean;
}

export type PhysicalKey =
  | 'keyA' | 'keyB' | 'keyC' | 'keyD' | 'keyE' | 'keyF' | 'keyG'
  | 'keyH' | 'keyI' | 'keyJ' | 'keyK' | 'keyL' | 'keyM' | 'keyN'
  | 'keyO' | 'keyP' | 'keyQ' | 'keyR' | 'keyS' | 'keyT' | 'keyU'
  | 'keyV' | 'keyW' | 'keyX' | 'keyY' | 'keyZ'
  | 'digit0' | 'digit1' | 'digit2' | 'digit3' | 'digit4'
  | 'digit5' | 'digit6' | 'digit7' | 'digit8' | 'digit9'
  | 'backquote' | 'backslash' | 'bracketLeft' | 'bracketRight'
  | 'comma' | 'equal' | 'minus' | 'period' | 'quote' | 'semicolon' | 'slash'
  | 'numpad0' | 'numpad1' | 'numpad2' | 'numpad3' | 'numpad4'
  | 'numpad5' | 'numpad6' | 'numpad7' | 'numpad8' | 'numpad9'
  | 'numpadAdd' | 'numpadDecimal' | 'numpadDivide' | 'numpadMultiply' | 'numpadSubtract'
  | 'numpadEnter' | 'numpadEqual'
  | 'intlBackslash' | 'intlRo' | 'intlYen'
  | 'printScreen' | 'pause' | 'capsLock' | 'numLock' | 'scrollLock' | 'contextMenu'
  | 'f1' | 'f2' | 'f3' | 'f4' | 'f5' | 'f6' | 'f7' | 'f8'
  | 'f9' | 'f10' | 'f11' | 'f12' | 'f13' | 'f14' | 'f15' | 'f16'
  | 'f17' | 'f18' | 'f19' | 'f20' | 'f21' | 'f22' | 'f23' | 'f24';

export type LogicalKey =
  | 'arrowUp' | 'arrowDown' | 'arrowLeft' | 'arrowRight'
  | 'backspace' | 'delete' | 'end' | 'enter' | 'escape' | 'home'
  | 'insert' | 'pageDown' | 'pageUp' | 'space' | 'tab';

export type KeyCode =
  | { kind: 'physical'; value: PhysicalKey }
  | { kind: 'logical'; value: LogicalKey };

export interface Chord {
  modifiers: Modifiers;
  key: KeyCode;
}

export type Trigger =
  | { type: 'chord'; chord: Chord }
  | { type: 'sequence'; steps: Chord[]; timeoutMs: 650 };

export type HotkeyAction =
  | 'translateSelection'
  | 'captureScreen'
  | 'translateImage'
  | 'openMainWindow';

export interface HotkeyBinding {
  id: string;
  trigger: Trigger;
  action: HotkeyAction;
  profileId: string;
  force: boolean;
}
