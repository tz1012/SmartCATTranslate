import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { CaptureWorkspace } from './CaptureWorkspace';
import type { CaptureJobResult } from './types';

const mocks = vi.hoisted(() => ({
  sourceReady: undefined as ((event: { payload: CaptureJobResult }) => void) | undefined,
  sessionEnded: undefined as ((event: { payload: { sessionId: string; status: 'failed' | 'cancelled'; reason?: string } }) => void) | undefined,
  listen: vi.fn(),
  startScreenCapture: vi.fn(),
  translateImage: vi.fn(),
}));

vi.mock('@tauri-apps/api/event', () => ({
  listen: mocks.listen,
}));

vi.mock('./captureApi', () => ({
  cancelImageTranslation: vi.fn(),
  chooseImage: vi.fn(),
  startScreenCapture: mocks.startScreenCapture,
  translateImage: mocks.translateImage,
}));

const sourceReady: CaptureJobResult = {
  jobId: 'job-1',
  status: 'sourceReady',
  imageWidth: 1,
  imageHeight: 1,
  translatedBlocks: [],
  warnings: [],
};

beforeEach(() => {
  mocks.listen.mockImplementation(async (name: string, handler: (event: { payload: unknown }) => void) => {
    if (name === 'capture-source-ready') mocks.sourceReady = handler as typeof mocks.sourceReady;
    if (name === 'capture-session-ended') mocks.sessionEnded = handler as typeof mocks.sessionEnded;
    return vi.fn();
  });
  mocks.startScreenCapture.mockResolvedValue({ sessionId: 'session-1', monitors: [] });
  mocks.translateImage.mockRejectedValue('translation_tool_rejected');
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  mocks.sourceReady = undefined;
  mocks.sessionEnded = undefined;
});

it('keeps the source-ready retry action and reports a safe translation error', async () => {
  render(<CaptureWorkspace locale="en" />);
  await waitFor(() => expect(mocks.sourceReady).toBeDefined());
  act(() => mocks.sourceReady?.({ payload: sourceReady }));

  fireEvent.click(screen.getByRole('button', { name: 'Start OCR & translation' }));

  const alert = await screen.findByRole('alert');
  expect(screen.getByRole('button', { name: 'Start OCR & translation' })).toBeEnabled();
  expect(alert.textContent).toBe(
    'Could not complete image translation. Support code: translation_tool_rejected',
  );
  expect(alert).not.toHaveTextContent('Could not start capture');
});

it.each([
  ['failed', 'screen_capture_failed'],
  ['cancelled', undefined],
] as const)('clears workspace busy when a future capture-session-ended event is %s', async (status, reason) => {
  render(<CaptureWorkspace locale="en" />);
  await waitFor(() => expect(mocks.sessionEnded).toBeDefined());

  fireEvent.click(screen.getByRole('button', { name: 'Select screen region' }));
  await waitFor(() => expect(screen.getByRole('button', { name: 'Select screen region' })).toBeDisabled());

  act(() => mocks.sessionEnded?.({
    payload: { sessionId: 'session-1', status, ...(reason ? { reason } : {}) },
  }));

  expect(screen.getByRole('button', { name: 'Select screen region' })).toBeEnabled();
  expect(screen.queryByRole('status')).not.toBeInTheDocument();
});

it('handles a failed capture-session-ended event before the pending start resolves', async () => {
  let resolveStart!: (value: { sessionId: string; monitors: [] }) => void;
  mocks.startScreenCapture.mockReturnValue(new Promise((resolve) => { resolveStart = resolve; }));
  render(<CaptureWorkspace locale="en" />);
  await waitFor(() => expect(mocks.sessionEnded).toBeDefined());

  fireEvent.click(screen.getByRole('button', { name: 'Select screen region' }));
  await waitFor(() => expect(mocks.startScreenCapture).toHaveBeenCalledTimes(1));
  expect(screen.getByRole('button', { name: 'Select screen region' })).toBeDisabled();

  act(() => mocks.sessionEnded?.({
    payload: { sessionId: 'session-1', status: 'failed', reason: 'invalid_capture_selection' },
  }));
  await act(async () => resolveStart({ sessionId: 'session-1', monitors: [] }));

  await waitFor(() => expect(screen.getByRole('button', { name: 'Select screen region' })).toBeEnabled());
  expect(screen.queryByRole('status')).not.toBeInTheDocument();
  expect(screen.getByRole('alert')).toHaveTextContent(
    /^Could not start screen capture\. Support code: invalid_capture_selection$/,
  );
});

it('keeps capture start disabled until both capture listeners are installed', async () => {
  const listenerResolvers: Array<(unlisten: () => void) => void> = [];
  mocks.listen.mockImplementation((name: string, handler: (event: { payload: unknown }) => void) => {
    if (name === 'capture-source-ready') mocks.sourceReady = handler as typeof mocks.sourceReady;
    if (name === 'capture-session-ended') mocks.sessionEnded = handler as typeof mocks.sessionEnded;
    return new Promise((resolve) => listenerResolvers.push(resolve));
  });

  render(<CaptureWorkspace locale="en" />);
  const start = screen.getByRole('button', { name: 'Select screen region' });
  expect(start).toBeDisabled();
  await waitFor(() => expect(listenerResolvers).toHaveLength(3));

  await act(async () => listenerResolvers[0](vi.fn()));
  expect(start).toBeDisabled();
  await act(async () => listenerResolvers[1](vi.fn()));
  expect(start).toBeEnabled();
  await act(async () => listenerResolvers[2](vi.fn()));
});

it('does not restore a phantom active session when source-ready precedes start resolution', async () => {
  let resolveStart!: (value: { sessionId: string; monitors: [] }) => void;
  mocks.startScreenCapture.mockReturnValue(new Promise((resolve) => { resolveStart = resolve; }));
  render(<CaptureWorkspace locale="en" />);
  await waitFor(() => expect(screen.getByRole('button', { name: 'Select screen region' })).toBeEnabled());

  fireEvent.click(screen.getByRole('button', { name: 'Select screen region' }));
  act(() => mocks.sourceReady?.({
    payload: { ...sourceReady, captureSessionId: 'session-1' } as CaptureJobResult,
  }));
  await act(async () => resolveStart({ sessionId: 'session-1', monitors: [] }));

  expect(screen.getByText('Image is ready.')).toBeInTheDocument();
  act(() => mocks.sessionEnded?.({
    payload: { sessionId: 'session-1', status: 'failed', reason: 'screen_capture_failed' },
  }));
  expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  expect(screen.getByText('Image is ready.')).toBeInTheDocument();
});

it('ignores a stale pending terminal event when a different session starts', async () => {
  let resolveStart!: (value: { sessionId: string; monitors: [] }) => void;
  mocks.startScreenCapture.mockReturnValue(new Promise((resolve) => { resolveStart = resolve; }));
  render(<CaptureWorkspace locale="en" />);
  await waitFor(() => expect(screen.getByRole('button', { name: 'Select screen region' })).toBeEnabled());

  fireEvent.click(screen.getByRole('button', { name: 'Select screen region' }));
  act(() => mocks.sessionEnded?.({
    payload: { sessionId: 'session-a', status: 'cancelled' },
  }));
  await act(async () => resolveStart({ sessionId: 'session-b', monitors: [] }));

  expect(screen.getByRole('button', { name: 'Select screen region' })).toBeDisabled();
  act(() => mocks.sessionEnded?.({
    payload: { sessionId: 'session-b', status: 'cancelled' },
  }));
  expect(screen.getByRole('button', { name: 'Select screen region' })).toBeEnabled();
});
