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
});
