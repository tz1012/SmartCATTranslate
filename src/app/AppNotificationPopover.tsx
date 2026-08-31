import { useEffect, useRef } from 'react';

export function AppNotificationPopover({
  label,
  notifications,
  onClose,
}: {
  label: string;
  notifications: string[];
  onClose: () => void;
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
      <ul>{notifications.map((notification) => <li key={notification}>{notification}</li>)}</ul>
    </aside>
  );
}
