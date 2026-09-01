import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { CaptureWorkspace } from './CaptureWorkspace';
import type { CaptureJobResult } from './types';

const mocks = vi.hoisted(() => ({
  sourceReady: undefined as ((event: { payload: CaptureJobResult }) => void) | undefined,
  translateImage: vi.fn(),
}));

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (name: string, handler: (event: { payload: CaptureJobResult }) => void) => {
    if (name === 'capture-source-ready') mocks.sourceReady = handler;
    return vi.fn();
  }),
}));

vi.mock('./captureApi', () => ({
  cancelImageTranslation: vi.fn(),
  chooseImage: vi.fn(),
  startScreenCapture: vi.fn(),
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
  mocks.translateImage.mockRejectedValue('translation_tool_rejected');
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  mocks.sourceReady = undefined;
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
