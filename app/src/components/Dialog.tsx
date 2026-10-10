import { useEffect, useId, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type ReactNode } from "react";
import { t } from "../i18n";
import { tabTarget } from "../lib/focus";
import { CloseIcon } from "./Icons";

const FOCUSABLE = [
  "button:not([disabled])", "[href]", "input:not([disabled])", "select:not([disabled])", "textarea:not([disabled])",
  "[tabindex]:not([tabindex=\"-1\"])"
].join(",");

/** Modal dialog frame: title bar with close button, Esc closes, Tab stays inside, focus returns to the opener on close. */
export function Dialog(props: {
  className: string;
  title: string;
  subtitle?: string;
  onClose: () => void;
  children: ReactNode;
}) {
  const titleId = useId();
  const ref = useRef<HTMLElement>(null);
  const { onClose } = props;

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  // Read while rendering: once the dialog is committed the page behind is inert and has lost the focus.
  const [opener] = useState(() => (document.activeElement instanceof HTMLElement ? document.activeElement : null));
  useEffect(() => {
    if (!ref.current?.contains(document.activeElement)) ref.current?.focus();
    return () => opener?.focus();
  }, [opener]);

  // The page behind is inert while a dialog is open (App); this keeps Tab inside when inert is unavailable.
  const onKeyDown = (e: ReactKeyboardEvent<HTMLElement>) => {
    if (e.key !== "Tab" || e.altKey || e.ctrlKey || e.metaKey) return;
    const root = ref.current;
    if (!root) return;
    const items = Array.from(root.querySelectorAll<HTMLElement>(FOCUSABLE)).filter((el) => el.getClientRects().length > 0);
    const target = tabTarget(items.length, items.indexOf(document.activeElement as HTMLElement), e.shiftKey);
    if (target === null) return;
    e.preventDefault();
    items[target]?.focus();
  };

  return (
    <div className="overlay">
      <section
        ref={ref}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        tabIndex={-1}
        className={`dialog ${props.className}`}
        onKeyDown={onKeyDown}
      >
        <div className="dialog-header">
          <span id={titleId} className="title">{props.title}</span>
          {props.subtitle && <span className="subtitle">{props.subtitle}</span>}
          <span className="spacer" />
          <button type="button" className="dialog-close" aria-label={t("common.close")} onClick={onClose}>
            <CloseIcon size={14} />
          </button>
        </div>
        {props.children}
      </section>
    </div>
  );
}
