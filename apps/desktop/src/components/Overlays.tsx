import { useEffect, useLayoutEffect, useRef, useState, useSyncExternalStore } from "react";
import { Icon } from "./Icon";

export type MenuItem = { label: string; icon?: string; run: () => void; danger?: boolean; disabled?: boolean; hint?: string } | "-";

interface Ask {
  title: string;
  body?: string;
  ok?: string;
  danger?: boolean;
  notice?: boolean;
}

type State = {
  dialog: (Ask & { resolve: (ok: boolean) => void }) | null;
  menu: { x: number; y: number; items: MenuItem[] } | null;
};

let state: State = { dialog: null, menu: null };
const subs = new Set<() => void>();
function set(s: Partial<State>) {
  state = { ...state, ...s };
  subs.forEach((f) => f());
}
const subscribe = (f: () => void) => {
  subs.add(f);
  return () => subs.delete(f);
};

export function ask(a: Ask): Promise<boolean> {
  return new Promise((resolve) => {
    state.dialog?.resolve(false);
    set({ dialog: { ...a, resolve }, menu: null });
  });
}

export function fail(e: unknown) {
  const text = e instanceof Error ? e.message : typeof e === "string" ? e : (e as { message?: string })?.message ?? String(e);
  ask({ title: "That didn't work", body: text, notice: true });
}

export function menu(e: React.MouseEvent, items: MenuItem[]) {
  e.preventDefault();
  e.stopPropagation();
  set({ menu: { x: e.clientX, y: e.clientY, items } });
}

function Dialog({ d }: { d: NonNullable<State["dialog"]> }) {
  const okRef = useRef<HTMLButtonElement>(null);
  const close = (ok: boolean) => {
    set({ dialog: null });
    d.resolve(ok);
  };
  useEffect(() => {
    okRef.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        close(false);
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [d]);

  return (
    <div className="fixed inset-0 z-[90] grid place-items-center bg-[rgba(4,6,12,.62)] p-6" onMouseDown={(e) => e.target === e.currentTarget && close(false)}>
      <div role="alertdialog" aria-modal="true" aria-labelledby="ov-title" className="page card w-full max-w-[440px] overflow-hidden shadow-[var(--shadow)]">
        <div className="flex gap-4 p-6">
          <span
            className={`grid h-10 w-10 shrink-0 place-items-center rounded-xl ring-1 ${
              d.danger || d.notice ? "bg-bad/12 text-bad ring-bad/25" : "bg-cyan/12 text-cyan ring-cyan/25"
            }`}
          >
            <Icon name={d.notice ? "warn" : d.danger ? "trash" : "info"} className="!h-[18px] !w-[18px]" />
          </span>
          <div className="min-w-0 flex-1">
            <h2 id="ov-title" className="text-[17px] font-bold">
              {d.title}
            </h2>
            {d.body && <p className="selectable mt-1.5 text-[13.5px] leading-relaxed break-words text-ink-2">{d.body}</p>}
          </div>
        </div>
        <div className="flex justify-end gap-2 border-t border-line bg-bg-2/60 px-6 py-4">
          {!d.notice && (
            <button className="btn sm" onClick={() => close(false)}>
              Cancel
            </button>
          )}
          <button ref={okRef} className={`btn sm ${d.danger ? "danger-solid" : "go"}`} onClick={() => close(true)}>
            {d.ok ?? "OK"}
          </button>
        </div>
      </div>
    </div>
  );
}

function Menu({ m }: { m: NonNullable<State["menu"]> }) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ left: m.x, top: m.y });
  const [active, setActive] = useState(-1);
  const close = () => set({ menu: null });
  const live = m.items.map((it, i) => (it !== "-" && !it.disabled ? i : -1)).filter((i) => i >= 0);

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    const left = m.x + r.width > window.innerWidth - 8 ? Math.max(8, m.x - r.width) : m.x;
    const top = m.y + r.height > window.innerHeight - 8 ? Math.max(8, m.y - r.height) : m.y;
    setPos({ left, top });
    el.focus();
  }, [m]);

  useEffect(() => {
    const away = (e: Event) => {
      if (!ref.current?.contains(e.target as Node)) close();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") return close();
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        const k = live.indexOf(active);
        const next = e.key === "ArrowDown" ? live[(k + 1) % live.length] : live[(k - 1 + live.length) % live.length];
        setActive(next ?? -1);
      }
      if (e.key === "Enter" && active >= 0) {
        e.preventDefault();
        const it = m.items[active];
        close();
        if (it !== "-") it.run();
      }
    };
    window.addEventListener("mousedown", away, true);
    window.addEventListener("wheel", close, { passive: true });
    window.addEventListener("blur", close);
    window.addEventListener("resize", close);
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("mousedown", away, true);
      window.removeEventListener("wheel", close);
      window.removeEventListener("blur", close);
      window.removeEventListener("resize", close);
      window.removeEventListener("keydown", onKey, true);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [m, active]);

  return (
    <div
      ref={ref}
      role="menu"
      tabIndex={-1}
      className="ctx fixed z-[95] min-w-[210px] rounded-xl border border-line-2 bg-card p-1.5 shadow-[var(--shadow)] outline-none"
      style={pos}
      onContextMenu={(e) => e.preventDefault()}
    >
      {m.items.map((it, i) =>
        it === "-" ? (
          <div key={i} className="mx-2 my-1 h-px bg-line" />
        ) : (
          <button
            key={i}
            role="menuitem"
            disabled={it.disabled}
            className={`flex h-8 w-full items-center gap-2.5 rounded-lg px-2.5 text-left text-[13.5px] transition-colors disabled:opacity-40 ${
              it.danger ? "text-bad" : "text-ink"
            } ${active === i ? (it.danger ? "bg-bad/12" : "bg-hi") : ""}`}
            onMouseEnter={() => setActive(i)}
            onMouseLeave={() => setActive(-1)}
            onClick={() => {
              close();
              it.run();
            }}
          >
            <Icon name={it.icon ?? ""} className={`!h-[15px] !w-[15px] shrink-0 ${it.danger ? "" : "text-ink-3"}`} />
            <span className="flex-1 truncate">{it.label}</span>
            {it.hint && <span className="font-mono text-[11px] text-ink-3">{it.hint}</span>}
          </button>
        ),
      )}
    </div>
  );
}

export function Overlays() {
  const s = useSyncExternalStore(subscribe, () => state);

  useEffect(() => {
    const onMenu = (e: MouseEvent) => {
      const t = e.target as HTMLElement | null;
      const editable = t?.closest("input, textarea, [contenteditable='true']");
      const selected = !!window.getSelection()?.toString();
      if (editable || selected || (import.meta.env.DEV && e.shiftKey)) return;
      e.preventDefault();
    };
    window.addEventListener("contextmenu", onMenu);
    const demo = import.meta.env.DEV ? new URLSearchParams(location.hash.slice(1)).get("demo") : null;
    const t = window.setInterval(() => {
      const card = document.querySelector<HTMLElement>(".discover-layer .tool");
      if (!demo || !card) return;
      window.clearInterval(t);
      const r = card.getBoundingClientRect();
      if (demo === "menu") card.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: r.left + r.width * 0.6, clientY: r.top + r.height * 0.4 }));
      if (demo === "ask") ask({ title: "Uninstall ZPad?", body: "It is removed from this computer, with its menu entry and shortcuts. Your own files made with it stay.", ok: "Uninstall", danger: true });
    }, 100);
    return () => {
      window.removeEventListener("contextmenu", onMenu);
      window.clearInterval(t);
    };
  }, []);

  return (
    <>
      {s.menu && <Menu m={s.menu} />}
      {s.dialog && <Dialog d={s.dialog} />}
    </>
  );
}
