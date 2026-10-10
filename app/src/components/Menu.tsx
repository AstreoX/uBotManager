import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { t } from "../i18n";

export interface MenuEntry {
  kind?: "item" | "separator" | "section";
  label?: string;
  hint?: string;
  /** Tooltip, mostly to say why an entry is disabled. */
  title?: string;
  disabled?: boolean;
  onSelect?: () => void;
}

/** The ⋮ button of a tool window with its popup menu. */
export function OptionsMenu(props: { entries: MenuEntry[]; label?: string }) {
  const [anchor, setAnchor] = useState<DOMRect | null>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);
  return (
    <>
      <button
        ref={buttonRef}
        type="button"
        className={`menu-button${anchor ? " open" : ""}`}
        aria-label={props.label ?? t("common.options")}
        aria-haspopup="menu"
        aria-expanded={anchor !== null}
        onClick={() => setAnchor(anchor ? null : buttonRef.current?.getBoundingClientRect() ?? null)}
      >
        ⋮
      </button>
      {anchor && <PopupMenu anchor={anchor} entries={props.entries} onClose={() => setAnchor(null)} ignore={buttonRef.current} />}
    </>
  );
}

function PopupMenu(props: { anchor: DOMRect; entries: MenuEntry[]; onClose: () => void; ignore?: HTMLElement | null }) {
  const ref = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState({ left: props.anchor.right, top: props.anchor.bottom + 4 });
  const { onClose, ignore } = props;

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const width = el.offsetWidth;
    const height = el.offsetHeight;
    const left = Math.max(8, Math.min(props.anchor.right - width, window.innerWidth - width - 8));
    const below = props.anchor.bottom + 4;
    const top = below + height > window.innerHeight - 8 ? Math.max(8, props.anchor.top - height - 4) : below;
    setPosition({ left, top });
  }, [props.anchor]);

  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      const target = e.target as Node;
      if (ref.current?.contains(target) || ignore?.contains(target)) return;
      onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopImmediatePropagation();
        onClose();
      }
    };
    document.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("blur", onClose);
    return () => {
      document.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("blur", onClose);
    };
  }, [onClose, ignore]);

  return (
    <div ref={ref} className="menu" role="menu" style={{ left: position.left, top: position.top }}>
      {props.entries.map((entry, i): ReactNode => {
        if (entry.kind === "separator") return <div key={i} className="menu-separator" role="separator" />;
        if (entry.kind === "section") return <div key={i} className="menu-section">{entry.label}</div>;
        return (
          <button
            key={i}
            type="button"
            role="menuitem"
            className="menu-item"
            title={entry.title}
            disabled={entry.disabled}
            onClick={() => {
              onClose();
              entry.onSelect?.();
            }}
          >
            <span>{entry.label}</span>
            {entry.hint && <span className="hint">{entry.hint}</span>}
          </button>
        );
      })}
    </div>
  );
}
