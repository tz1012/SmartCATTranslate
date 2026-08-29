import { cleanup, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { App } from './App';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn().mockResolvedValue(() => undefined) }));

afterEach(cleanup);

describe('App', () => {
  it('mounts the complete text translation workspace', async () => {
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'get_settings') return {
        schemaVersion: 1, locale: 'ko', theme: 'system', defaultProfileId: 'default-profile',
        profiles: [{ id: 'default-profile', name: '기본 프로필', field: 'general', profile: { sourceLanguage: null, targetLanguage: 'ko', quality: 'balanced', tone: 'natural', protectedTerms: [] } }],
        glossary: [], selectedModel: { type: 'automatic' }, launchAtLogin: false, closeBehavior: 'keepInTray', quickAccessPosition: 'popup', historyRetentionDays: 30,
      };
      if (command === 'get_account') return { account: { state: 'signedOut' }, loginPending: false };
      throw new Error(`unexpected command: ${command}`);
    });
    render(<App />);
    expect(screen.getByRole('heading', { name: 'SmartCAT Translate' })).toBeVisible();
    expect(await screen.findByLabelText('원문')).toBeVisible();
    expect(screen.getByLabelText('번역문')).toBeVisible();
    expect(screen.getByRole('tab', { name: '텍스트' })).toHaveAttribute('aria-selected', 'true');
  });

  it('shows the text translation workspace in English', async () => {
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'get_settings') return {
        schemaVersion: 1, locale: 'en', theme: 'system', defaultProfileId: 'default-profile',
        profiles: [{ id: 'default-profile', name: 'Default profile', field: 'general', profile: { sourceLanguage: null, targetLanguage: 'ko', quality: 'balanced', tone: 'natural', protectedTerms: [] } }],
        glossary: [], selectedModel: { type: 'automatic' }, launchAtLogin: false, closeBehavior: 'keepInTray', quickAccessPosition: 'popup', historyRetentionDays: 30,
      };
      if (command === 'get_account') return { account: { state: 'signedOut' }, loginPending: false };
      throw new Error(`unexpected command: ${command}`);
    });
    const { container } = render(<App locale="en" />);
    expect(await screen.findByRole('region', { name: 'Text translation' })).toBeVisible();
    expect(screen.getByLabelText('Source text')).toBeVisible();
    expect(screen.getByLabelText('Translation')).toBeVisible();
    expect(screen.getByRole('complementary', { name: 'ChatGPT account' })).toBeVisible();
    expect(container.textContent).not.toMatch(/[\u3131-\u318E\uAC00-\uD7A3]/u);
    for (const element of container.querySelectorAll('[aria-label]')) {
      expect(element.getAttribute('aria-label')).not.toMatch(/[\u3131-\u318E\uAC00-\uD7A3]/u);
    }
  });

  it('uses the saved interface locale when no locale override is provided', async () => {
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'get_settings') return {
        schemaVersion: 1, locale: 'en', theme: 'system', defaultProfileId: 'default-profile',
        profiles: [{ id: 'default-profile', name: 'Default profile', field: 'general', profile: { sourceLanguage: null, targetLanguage: 'ko', quality: 'balanced', tone: 'natural', protectedTerms: [] } }],
        glossary: [], selectedModel: { type: 'automatic' }, launchAtLogin: false, closeBehavior: 'keepInTray', quickAccessPosition: 'popup', historyRetentionDays: 30,
      };
      if (command === 'get_account') return { account: { state: 'signedOut' }, loginPending: false };
      throw new Error(`unexpected command: ${command}`);
    });

    render(<App />);

    expect(await screen.findByRole('region', { name: 'Text translation' })).toBeVisible();
    expect(screen.getByRole('complementary', { name: 'ChatGPT account' })).toBeVisible();
  });

  it('mounts the settings screen through nontechnical accessible navigation', async () => {
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'get_settings') return {
        schemaVersion: 1, locale: 'ko', theme: 'dark', defaultProfileId: 'default-profile',
        profiles: [{ id: 'default-profile', name: '기본 프로필', field: 'general', profile: { sourceLanguage: null, targetLanguage: 'ko', quality: 'balanced', tone: 'natural', protectedTerms: [] } }],
        glossary: [], selectedModel: { type: 'automatic' }, launchAtLogin: false, closeBehavior: 'keepInTray', quickAccessPosition: 'popup', historyRetentionDays: 30,
      };
      if (command === 'get_account') return { account: { state: 'signedOut' }, loginPending: false };
      if (command === 'get_rate_limits') return { primaryUsedPercent: null, primaryResetsAt: null, secondaryUsedPercent: null, secondaryResetsAt: null };
      if (command === 'list_available_models') return [];
      throw new Error(`unexpected command: ${command}`);
    });
    const { container } = render(<App />);
    const translateTab = await screen.findByRole('tab', { name: '번역' });
    const settingsTab = screen.getByRole('tab', { name: '설정' });
    expect(translateTab).toHaveAttribute('aria-controls', 'app-panel-translate');
    expect(settingsTab).toHaveAttribute('aria-controls', 'app-panel-settings');
    expect(container.querySelector('main')).toHaveAttribute('data-theme', 'dark');

    await userEvent.click(settingsTab);
    expect(screen.getByRole('tabpanel', { name: '설정' })).toBeVisible();
    expect(screen.getByRole('heading', { name: '설정' })).toBeVisible();
    expect(screen.getByLabelText('번역 프로필')).toBeVisible();
  });
});
