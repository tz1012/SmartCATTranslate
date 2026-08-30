import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { App } from './app/App';
import { QuickPopup } from './features/translation/QuickPopup';
import { getCurrentWindow } from '@tauri-apps/api/window';
import './styles.css';

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    {getCurrentWindow().label === 'quick-popup' ? <QuickPopup /> : <App />}
  </StrictMode>,
);
