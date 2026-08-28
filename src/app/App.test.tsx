import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { App } from './App';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn().mockResolvedValue(() => undefined) }));

afterEach(cleanup);

describe('App', () => {
  it('shows the text translation workspace', () => {
    render(<App />);
    expect(screen.getByRole('heading', { name: 'SmartCAT Translate' })).toBeVisible();
    expect(screen.getByLabelText('원문')).toBeVisible();
    expect(screen.getByLabelText('번역문')).toBeVisible();
  });

  it('shows the text translation workspace in English', () => {
    vi.mocked(invoke).mockResolvedValue({
      account: { state: 'signedOut' },
      loginPending: false,
    });
    const { container } = render(<App locale="en" />);
    expect(screen.getByRole('region', { name: 'Text translation' })).toBeVisible();
    expect(screen.getByLabelText('Source text')).toBeVisible();
    expect(screen.getByLabelText('Translation')).toBeVisible();
    expect(screen.getByRole('complementary', { name: 'ChatGPT account' })).toBeVisible();
    expect(container.textContent).not.toMatch(/[\u3131-\u318E\uAC00-\uD7A3]/u);
    for (const element of container.querySelectorAll('[aria-label]')) {
      expect(element.getAttribute('aria-label')).not.toMatch(/[\u3131-\u318E\uAC00-\uD7A3]/u);
    }
  });
});
