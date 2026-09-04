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
  notifications: (count: number) => string;
};

export function AppTopBar({
  activeView,
  labels,
  menuOpen,
  menuButtonRef,
  notificationCount,
  notificationsOpen,
  statusMessage,
  onNavigate,
  onToggleNotifications,
  onToggleMenu,
}: {
  activeView: AppView;
  labels: AppTopBarLabels;
  menuOpen: boolean;
  menuButtonRef: RefObject<HTMLButtonElement | null>;
  notificationCount: number;
  notificationsOpen: boolean;
  statusMessage?: string;
  onNavigate: (view: AppView) => void;
  onToggleNotifications: () => void;
  onToggleMenu: () => void;
}) {
  const tabs: Array<[Exclude<AppView, 'settings'>, string, string]> = [
    ['translate', labels.translate, '文A'],
    ['capture', labels.capture, '▣'],
    ['documents', labels.documents, '▤'],
    ['history', labels.history, '◷'],
  ];

  return (
    <header className={`app-shell-header${statusMessage ? ' has-status' : ''}`}>
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
      <div className="app-primary-navigation">
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
      </div>
      {statusMessage && <span id="settings-navigation-status" className="app-navigation-note" role="status">{statusMessage}</span>}
      <button
        type="button"
        className="app-notification-trigger"
        aria-label={labels.notifications(notificationCount)}
        aria-controls="app-notification-popover"
        aria-expanded={notificationsOpen}
        onClick={onToggleNotifications}
      >
        <svg aria-hidden="true" viewBox="0 0 24 24" width="21" height="21">
          <path d="M18 8a6 6 0 0 0-12 0c0 7-3 7-3 9h18c0-2-3-2-3-9M10 21h4" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
        {notificationCount > 0 && <i aria-hidden="true">{notificationCount}</i>}
      </button>
    </header>
  );
}
