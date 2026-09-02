import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { HotkeyRecorder } from './HotkeyRecorder';

const invoke = vi.fn();

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invoke(...args),
}));

describe('HotkeyRecorder', () => {
  beforeEach(() => {
    invoke.mockResolvedValue(undefined);
  });

  afterEach(cleanup);

  it('moves focus to the recorder before accepting the shortcut', async () => {
    render(<HotkeyRecorder locale="en" value={null} onChange={vi.fn()} />);

    await userEvent.click(screen.getByRole('button', { name: 'Record shortcut' }));

    expect(screen.getByRole('group', { name: 'Record shortcut' })).toHaveFocus();
  });

  it('captures a chord while its modifiers are held even when they are released first', () => {
    const onChange = vi.fn();
    render(<HotkeyRecorder locale="en" value={null} onChange={onChange} />);

    fireEvent.click(screen.getByRole('button', { name: 'Record shortcut' }));
    const recorder = screen.getByRole('group', { name: 'Record shortcut' });
    fireEvent.keyDown(recorder, {
      key: 't',
      code: 'KeyT',
      ctrlKey: true,
      altKey: true,
      shiftKey: true,
    });
    fireEvent.keyUp(recorder, { key: 'Control', code: 'ControlLeft' });
    fireEvent.keyUp(recorder, { key: 'Alt', code: 'AltLeft' });
    fireEvent.keyUp(recorder, { key: 'Shift', code: 'ShiftLeft' });
    fireEvent.keyUp(recorder, { key: 't', code: 'KeyT' });

    expect(screen.getByText('Ctrl+Alt+Shift+T')).toBeVisible();
    fireEvent.click(screen.getByRole('button', { name: 'Done' }));
    expect(onChange).toHaveBeenCalledWith({
      type: 'chord',
      chord: {
        modifiers: { ctrl: true, alt: true, shift: true, meta: false },
        key: { kind: 'physical', value: 'keyT' },
      },
    });
  });

  it('notifies its owner before starting a new recording', () => {
    const onRecordingStart = vi.fn();
    render(
      <HotkeyRecorder
        locale="en"
        value={null}
        onChange={vi.fn()}
        onRecordingStart={onRecordingStart}
      />,
    );

    fireEvent.click(screen.getByRole('button', { name: 'Record shortcut' }));

    expect(onRecordingStart).toHaveBeenCalledOnce();
  });
});
