import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { SecretModeSwitch } from './secretMode';

afterEach(cleanup);

describe('SecretModeSwitch', () => {
  it('renders a compact option without the large persistence description', () => {
    render(<SecretModeSwitch locale="ko" value={false} onChange={vi.fn()} compact />);

    expect(screen.getByText('시크릿 번역').closest('label')).toHaveClass('secret-mode-switch-compact');
    expect(screen.queryByText('로컬 암호화 기록 사용')).not.toBeInTheDocument();
  });
});
