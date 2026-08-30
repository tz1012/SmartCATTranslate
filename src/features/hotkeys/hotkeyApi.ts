import { invoke } from '@tauri-apps/api/core';
import type { HotkeyBinding, Trigger } from './types';

export type ConflictLevel = 'none' | 'possible' | 'confirmed';
export type ConflictSeverity = 'advisory' | 'warning' | 'blocking';

export interface ConflictCause {
  severity: ConflictSeverity;
  description: string;
  application: string | null;
  feature: string | null;
  sourceUrl: string | null;
  verifiedAt: string | null;
}

export interface ConflictReport {
  level: ConflictLevel;
  causes: ConflictCause[];
  alternatives: Trigger[];
  canForce: boolean;
}

export interface BlockedApp {
  platform: 'windows' | 'macos';
  executable: string | null;
  bundleId: string | null;
  catalogName: string | null;
}

export const analyzeHotkey = (trigger: Trigger) => invoke<ConflictReport>('analyze_hotkey', { trigger });
export const listHotkeys = () => invoke<HotkeyBinding[]>('list_hotkeys');
export const saveHotkey = (binding: HotkeyBinding) => invoke<HotkeyBinding[]>('save_hotkey', { binding });
export const suspendHotkeys = (suspended: boolean) => invoke<void>('suspend_hotkeys', { suspended });
export const listBlockedApps = () => invoke<BlockedApp[]>('list_blocked_apps');
export const saveBlockedApps = (blockedApps: BlockedApp[]) => invoke<BlockedApp[]>('save_blocked_apps', { blockedApps });

