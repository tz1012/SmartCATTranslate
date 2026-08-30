import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { App } from './app/App';
import { QuickPopup } from './features/translation/QuickPopup';
import { CaptureOverlay } from './features/capture/CaptureOverlay';
import { getCurrentWindow } from '@tauri-apps/api/window';
import './styles.css';

const isCaptureOverlay = new URLSearchParams(window.location.search).get('captureOverlay') === '1';

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    {isCaptureOverlay ? <CaptureOverlay /> : getCurrentWindow().label === 'quick-popup' ? <QuickPopup /> : <App />}
  </StrictMode>,
);
