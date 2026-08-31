import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { listen } from '@tauri-apps/api/event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { CaptureOverlay } from './CaptureOverlay';
import type { CaptureSelection, OverlayDescriptor } from './types';

const bridge = vi.hoisted(() => ({
  selectionHandler: undefined as ((event: { payload: { sessionId: string; selection: CaptureSelection } }) => void) | undefined,
  cancel: vi.fn(),
  complete: vi.fn(),
  focus: vi.fn(),
  getOverlay: vi.fn(),
  update: vi.fn(),
}));

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (_name: string, handler: typeof bridge.selectionHandler) => {
    bridge.selectionHandler = handler;
    return vi.fn();
  }),
}));

vi.mock('./captureApi', () => ({
  cancelScreenCapture: bridge.cancel,
  completeScreenCapture: bridge.complete,
  focusCaptureWindow: bridge.focus,
  getCaptureOverlay: bridge.getOverlay,
  updateScreenSelection: bridge.update,
}));

const descriptor: OverlayDescriptor = {
  sessionId: 'session-1',
  monitor: {
    id: 'monitor-1',
    name: 'Display 1',
    physicalBounds: { x: 0, y: 0, width: 1920, height: 1080 },
    logicalBounds: { x: 0, y: 0, width: 1920, height: 1080 },
    scaleFactor: 1,
    primary: true,
  },
  backgroundDataUrl: 'data:image/png;base64,AA==',
};

const validSelection: CaptureSelection = {
  globalPhysical: { x: 100, y: 120, width: 480, height: 260 },
};

beforeEach(() => {
  window.history.replaceState({}, '', '/?captureOverlay=1&session=session-1&monitor=monitor-1&locale=ko');
  bridge.getOverlay.mockResolvedValue(descriptor);
  bridge.cancel.mockResolvedValue(undefined);
  bridge.complete.mockResolvedValue(undefined);
  bridge.focus.mockResolvedValue(undefined);
  bridge.update.mockResolvedValue(undefined);
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  bridge.selectionHandler = undefined;
});

async function renderReadyOverlay() {
  render(<CaptureOverlay />);
  const overlay = await screen.findByRole('main', { name: '번역할 화면 영역 선택' });
  await waitFor(() => expect(listen).toHaveBeenCalledWith('capture-selection-updated', expect.any(Function)));
  return overlay;
}

describe('CaptureOverlay keyboard controls', () => {
  it('owns keyboard focus as soon as the overlay is ready', async () => {
    const overlay = await renderReadyOverlay();
    expect(overlay).toHaveFocus();
    expect(bridge.focus).toHaveBeenCalled();
  });

  it('confirms a valid selection with Enter', async () => {
    await renderReadyOverlay();
    act(() => bridge.selectionHandler?.({ payload: { sessionId: 'session-1', selection: validSelection } }));

    fireEvent.keyDown(document, { key: 'Enter', code: 'Enter' });

    expect(bridge.complete).toHaveBeenCalledWith('session-1', validSelection);
  });

  it('cancels with Escape before a selection exists', async () => {
    await renderReadyOverlay();

    fireEvent.keyDown(document, { key: 'Escape', code: 'Escape' });

    expect(bridge.cancel).toHaveBeenCalledWith('session-1');
  });

  it('provides clickable confirm and cancel fallbacks', async () => {
    await renderReadyOverlay();
    expect(screen.getByRole('button', { name: '확인' })).toBeDisabled();
    act(() => bridge.selectionHandler?.({ payload: { sessionId: 'session-1', selection: validSelection } }));
    fireEvent.click(screen.getByRole('button', { name: '확인' }));
    expect(bridge.complete).toHaveBeenCalledWith('session-1', validSelection);
    fireEvent.click(screen.getByRole('button', { name: '취소' }));
    expect(bridge.cancel).toHaveBeenCalledWith('session-1');
  });
});
