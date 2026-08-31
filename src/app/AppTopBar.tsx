import type { RefObject } from 'react';

export type AppView = 'translate' | 'capture' | 'documents' | 'history' | 'settings';

export type AppTopBarLabels = {
  navigation: string;
  translate: string;
  capture: string;
  documents: string;
  history: string;
  openMenu: string;
  closeMenu: string;
  accountMenu: string;
};

export function AppTopBar({
  activeView,
  labels,
  menuOpen,
  menuButtonRef,
  onNavigate,
  onToggleMenu,
}: {
  activeView: AppView;
  labels: AppTopBarLabels;
  menuOpen: boolean;
  menuButtonRef: RefObject<HTMLButtonElement | null>;
  onNavigate: (view: AppView) => void;
  onToggleMenu: () => void;
}) {
  const tabs: Array<[Exclude<AppView, 'settings'>, string, string]> = [
    ['translate', labels.translate, '文A'],
    ['capture', labels.capture, '▣'],
    ['documents', labels.documents, '▤'],
    ['history', labels.history, '◷'],
  ];

  return (
    <header className="app-shell-header">
      <button
        ref={menuButtonRef}
        type="button"
        className="app-menu-trigger"
        aria-label={menuOpen ? labels.closeMenu : labels.openMenu}
        aria-controls="app-menu-overlay"
        aria-expanded={menuOpen}
        onClick={onToggleMenu}
      >
        <span aria-hidden="true">☰</span>
      </button>
      <nav className="app-primary-tabs" role="tablist" aria-label={labels.navigation}>
        {tabs.map(([view, label, icon]) => (
          <button
            key={view}
            id={`app-tab-${view}`}
            type="button"
            role="tab"
            aria-selected={activeView === view}
            aria-controls={`app-panel-${view}`}
            onClick={() => onNavigate(view)}
          >
            <span aria-hidden="true">{icon}</span>
            {label}
          </button>
        ))}
      </nav>
      <button
        type="button"
        className="app-account-trigger"
        aria-label={labels.accountMenu}
        aria-controls="app-menu-overlay"
        aria-expanded={menuOpen}
        onClick={onToggleMenu}
      >
        <span aria-hidden="true">A</span>
        <i aria-hidden="true" />
      </button>
    </header>
  );
}
