import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { listen } from '@tauri-apps/api/event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import '../../styles.css';
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

function installPointerCapture(element: HTMLElement) {
  let capturedPointer: number | undefined;
  Object.defineProperties(element, {
    setPointerCapture: {
      configurable: true,
      value: vi.fn((pointerId: number) => { capturedPointer = pointerId; }),
    },
    hasPointerCapture: {
      configurable: true,
      value: vi.fn((pointerId: number) => capturedPointer === pointerId),
    },
    releasePointerCapture: {
      configurable: true,
      value: vi.fn((pointerId: number) => {
        if (capturedPointer === pointerId) capturedPointer = undefined;
      }),
    },
  });
}

function firePointer(element: HTMLElement, type: 'pointerDown' | 'pointerMove', init: { pointerId: number; clientX: number; clientY: number }) {
  const event = new Event(type === 'pointerDown' ? 'pointerdown' : 'pointermove', { bubbles: true });
  Object.defineProperties(event, {
    pointerId: { value: init.pointerId },
    clientX: { value: init.clientX },
    clientY: { value: init.clientY },
  });
  fireEvent(element, event);
}

describe('CaptureOverlay keyboard controls', () => {
  it('uses readable dark labels for the confirm and cancel buttons', async () => {
    await renderReadyOverlay();

    const buttons = screen.getAllByRole('button');
    expect(buttons.map((button) => getComputedStyle(button).color)).toEqual([
      'rgb(21, 34, 56)',
      'rgb(21, 34, 56)',
    ]);
  });

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

  it('maps a captured pointer beyond the overlay bounds for cross-monitor selection', async () => {
    bridge.getOverlay.mockResolvedValue({
      ...descriptor,
      monitor: {
        ...descriptor.monitor,
        physicalBounds: { x: -1600, y: 0, width: 1600, height: 900 },
        logicalBounds: { x: -1280, y: 0, width: 1280, height: 720 },
        scaleFactor: 1.25,
      },
    });
    const overlay = await renderReadyOverlay();
    installPointerCapture(overlay);
    vi.spyOn(overlay, 'getBoundingClientRect').mockReturnValue({
      x: 20,
      y: 40,
      left: 20,
      top: 40,
      right: 820,
      bottom: 490,
      width: 800,
      height: 450,
      toJSON: () => ({}),
    });

    firePointer(overlay, 'pointerDown', { pointerId: 7, clientX: 120, clientY: 90 });
    firePointer(overlay, 'pointerMove', { pointerId: 7, clientX: 1020, clientY: 540 });

    expect(bridge.update).toHaveBeenLastCalledWith('session-1', {
      globalPhysical: { x: -1400, y: 100, width: 1800, height: 900 },
    });
  });

  it('clips a cross-monitor selection only for local rendering', async () => {
    const overlay = await renderReadyOverlay();

    act(() => bridge.selectionHandler?.({
      payload: {
        sessionId: 'session-1',
        selection: { globalPhysical: { x: 1600, y: 900, width: 640, height: 360 } },
      },
    }));

    const localSelection = overlay.querySelector<HTMLElement>('.capture-selection');
    expect(localSelection).toHaveStyle({ left: '83.33333333333334%', top: '83.33333333333334%' });
    expect(localSelection).toHaveStyle({ width: '16.666666666666664%', height: '16.666666666666664%' });
  });

  it('deduplicates confirmation while completion is pending', async () => {
    bridge.complete.mockReturnValue(new Promise(() => undefined));
    await renderReadyOverlay();
    act(() => bridge.selectionHandler?.({ payload: { sessionId: 'session-1', selection: validSelection } }));

    fireEvent.keyDown(document, { key: 'Enter', code: 'Enter' });
    fireEvent.keyDown(document, { key: 'Enter', code: 'Enter' });

    expect(bridge.complete).toHaveBeenCalledTimes(1);
  });

  it('sanitizes completion rejection and keeps cancellation available', async () => {
    bridge.complete.mockRejectedValue(new Error('backend secret: C:\\Users\\operator\\capture.png'));
    await renderReadyOverlay();
    act(() => bridge.selectionHandler?.({ payload: { sessionId: 'session-1', selection: validSelection } }));

    fireEvent.keyDown(document, { key: 'Enter', code: 'Enter' });

    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent('screen_capture_completion_failed');
    expect(alert).not.toHaveTextContent('backend secret');
    const buttons = screen.getAllByRole('button');
    expect(buttons[1]).toBeEnabled();
    fireEvent.click(buttons[1]);
    expect(bridge.cancel).toHaveBeenCalledWith('session-1');
  });

  it('preserves invalid_capture_selection as the exact safe support code', async () => {
    bridge.complete.mockRejectedValue('invalid_capture_selection');
    await renderReadyOverlay();
    act(() => bridge.selectionHandler?.({ payload: { sessionId: 'session-1', selection: validSelection } }));

    fireEvent.keyDown(document, { key: 'Enter', code: 'Enter' });

    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent(/^invalid_capture_selection$/);
    expect(alert).not.toHaveTextContent('screen_capture_completion_failed');
  });
});
