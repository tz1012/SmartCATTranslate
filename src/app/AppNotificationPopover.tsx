import { useEffect, useRef } from 'react';

export type AppNotification = {
  id: string;
  message: string;
  actionLabel?: string;
  actionDisabled?: boolean;
  onAction?: () => void;
  status?: string;
};

export function AppNotificationPopover({
  label,
  dismissLabel,
  notifications,
  onClose,
  onDismiss,
}: {
  label: string;
  dismissLabel: string;
  notifications: AppNotification[];
  onClose: () => void;
  onDismiss: (id: string) => void;
}) {
  const panelRef = useRef<HTMLElement>(null);

  useEffect(() => {
    panelRef.current?.focus({ preventScroll: true });
    const handleKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.preventDefault();
        onClose();
      }
    };
    const handleOutside = (event: PointerEvent) => {
      const target = event.target;
      if (target instanceof Element && !panelRef.current?.contains(target) && !target.closest('[aria-controls="app-notification-popover"]')) onClose();
    };
    document.addEventListener('keydown', handleKey, true);
    document.addEventListener('pointerdown', handleOutside, true);
    return () => {
      document.removeEventListener('keydown', handleKey, true);
      document.removeEventListener('pointerdown', handleOutside, true);
    };
  }, [onClose]);

  return (
    <aside ref={panelRef} id="app-notification-popover" className="app-notification-popover" role="dialog" aria-label={label} tabIndex={-1}>
      <h2>{label}</h2>
      <ul>{notifications.map((notification) => <li key={notification.id}>
        <span>{notification.message}</span>
        <div className="app-notification-actions">
          {notification.onAction && notification.actionLabel && <button type="button" disabled={notification.actionDisabled} onClick={notification.onAction}>{notification.actionLabel}</button>}
          <button type="button" onClick={() => onDismiss(notification.id)}>{dismissLabel}</button>
        </div>
        {notification.status && <p role="status" aria-live="polite">{notification.status}</p>}
      </li>)}</ul>
    </aside>
  );
}
