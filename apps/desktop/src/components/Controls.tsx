import { ReactNode, useEffect, useId, useRef, useState } from "react";
import { Icon } from "./Icon";

export interface Option<T extends string | number> {
  value: T;
  label: string;
  hint?: string;
  icon?: ReactNode;
}

export function Select<T extends string | number>({
  value,
  options,
  onChange,
  className = "",
  size = "md",
  label,
}: {
  value: T;
  options: Option<T>[];
  onChange: (v: T) => void;
  className?: string;
  size?: "sm" | "md";
  label?: string;
}) {
  const [open, setOpen] = useState(false);
  const [focus, setFocus] = useState(0);
  const root = useRef<HTMLDivElement>(null);
  const id = useId();
  const current = options.find((o) => o.value === value) ?? options[0];

  useEffect(() => {
    if (!open) return;
    setFocus(Math.max(0, options.findIndex((o) => o.value === value)));
    const away = (e: MouseEvent) => {
      if (!root.current?.contains(e.target as Node)) setOpen(false);
    };
    window.addEventListener("mousedown", away);
    return () => window.removeEventListener("mousedown", away);
  }, [open, options, value]);

  const pick = (v: T) => {
    onChange(v);
    setOpen(false);
  };

  const onKey = (e: React.KeyboardEvent) => {
    if (!open && (e.key === "Enter" || e.key === " " || e.key === "ArrowDown")) {
      e.preventDefault();
      setOpen(true);
    } else if (open) {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        setOpen(false);
      } else if (e.key === "ArrowDown") {
        e.preventDefault();
        setFocus((f) => Math.min(options.length - 1, f + 1));
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        setFocus((f) => Math.max(0, f - 1));
      } else if (e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        pick(options[focus].value);
      }
    }
  };

  const h = size === "sm" ? "h-[30px] text-[13px] rounded-full pl-3 pr-2.5" : "h-[38px] text-[13.5px] rounded-[10px] pl-3.5 pr-3";
  return (
    <div ref={root} className={`relative ${className}`}>
      <button
        type="button"
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={id}
        aria-label={label}
        className={`flex w-full items-center gap-2 border bg-bg-2 font-medium text-ink transition-colors ${h} ${
          open ? "border-cyan/60 ring-4 ring-hi" : "border-line hover:border-line-2"
        }`}
        onClick={() => setOpen(!open)}
        onKeyDown={onKey}
      >
        {current?.icon}
        <span className="min-w-0 flex-1 truncate text-left">{current?.label}</span>
        <Icon name="back" className={`!h-3.5 !w-3.5 shrink-0 text-ink-3 transition-transform ${open ? "rotate-90" : "-rotate-90"}`} />
      </button>
      {open && (
        <div
          id={id}
          role="listbox"
          className="menu absolute right-0 z-40 mt-1.5 min-w-full overflow-hidden rounded-xl border border-line-2 bg-card p-1 shadow-[var(--shadow)]"
        >
          {options.map((o, i) => (
            <button
              key={String(o.value)}
              type="button"
              role="option"
              aria-selected={o.value === value}
              className={`flex w-full items-center gap-2.5 rounded-lg px-3 py-2 text-left text-[13.5px] whitespace-nowrap ${
                i === focus ? "bg-hi text-ink" : "text-ink-2"
              }`}
              onMouseEnter={() => setFocus(i)}
              onClick={() => pick(o.value)}
            >
              {o.icon}
              <span className="flex-1">
                <span className="block font-medium">{o.label}</span>
                {o.hint && <span className="block text-[12px] text-ink-3">{o.hint}</span>}
              </span>
              <Icon name="check" className={`!h-4 !w-4 text-cyan ${o.value === value ? "" : "invisible"}`} />
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

export function Toggle({ on, onChange, label }: { on: boolean; onChange: (v: boolean) => void; label?: string }) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      aria-label={label}
      className={`relative h-[26px] w-[46px] shrink-0 rounded-full border transition-colors duration-200 ${
        on ? "border-transparent bg-gradient-to-r from-cyan to-[color-mix(in_srgb,var(--cyan)_55%,var(--violet))]" : "border-line-2 bg-bg-2"
      }`}
      onClick={() => onChange(!on)}
    >
      <span
        className={`absolute top-1/2 h-[20px] w-[20px] -translate-y-1/2 rounded-full shadow-md transition-all duration-200 ${
          on ? "left-[22px] bg-white" : "left-[2px] bg-ink-3"
        }`}
      />
    </button>
  );
}

export function Segmented<T extends string | number>({ value, options, onChange }: { value: T; options: Option<T>[]; onChange: (v: T) => void }) {
  return (
    <div role="radiogroup" className="inline-flex rounded-[11px] border border-line bg-bg-2 p-[3px]">
      {options.map((o) => {
        const on = o.value === value;
        return (
          <button
            key={String(o.value)}
            type="button"
            role="radio"
            aria-checked={on}
            className={`flex h-[30px] items-center gap-1.5 rounded-lg px-3.5 text-[13px] font-medium whitespace-nowrap transition-all ${
              on ? "bg-card text-ink shadow-[0_1px_0_var(--line-2),0_4px_14px_-6px_rgba(0,0,0,.6)]" : "text-ink-3 hover:text-ink"
            }`}
            onClick={() => onChange(o.value)}
          >
            {o.icon}
            {o.label}
          </button>
        );
      })}
    </div>
  );
}
