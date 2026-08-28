import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { App } from './App';

describe('App', () => {
  it('shows the text translation workspace', () => {
    render(<App />);
    expect(screen.getByRole('heading', { name: 'SmartCAT Translate' })).toBeVisible();
    expect(screen.getByLabelText('원문')).toBeVisible();
    expect(screen.getByLabelText('번역문')).toBeVisible();
  });

  it('shows the text translation workspace in English', () => {
    render(<App locale="en" />);
    expect(screen.getByRole('region', { name: 'Text translation' })).toBeVisible();
    expect(screen.getByLabelText('Source text')).toBeVisible();
    expect(screen.getByLabelText('Translation')).toBeVisible();
  });
});
