import { KeyboardEvent, PointerEvent, ReactNode, useLayoutEffect, useRef, useState } from "react";
import { Settings } from "../api";
import { Flip, useEffectsLevel } from "../effects";
import { ALWAYS_SHOWN, Block, BLOCK_LABEL, blockHidden, blockOrder } from "../sidebar";
import { Toggle } from "./Controls";

interface Drag {
  id: Block;
  from: number;
  to: number;
  startY: number;
  step: number;
}

function Grip() {
  return (
    <svg viewBox="0 0 10 16" className="h-4 w-2.5" fill="currentColor" aria-hidden>
      {[2, 8, 14].map((y) => (
        <g key={y}>
          <circle cx="2" cy={y} r="1.4" />
          <circle cx="8" cy={y} r="1.4" />
        </g>
      ))}
    </svg>
  );
}

export function SidebarBlocks({ settings, save, icons }: { settings: Settings; save: (s: Settings) => void; icons: Record<Block, ReactNode> }) {
  const order = blockOrder(settings.sidebar);
  const list = useRef<HTMLDivElement>(null);
  const [drag, setDrag] = useState<Drag | null>(null);
  const settle = useRef<Flip.FlipState | null>(null);
  const { motion } = useEffectsLevel();

  const rows = () => (list.current ? [...list.current.querySelectorAll<HTMLElement>("[data-block]")] : []);

  useLayoutEffect(() => {
    const state = settle.current;
    settle.current = null;
    if (!state) return;
    rows().forEach((r) => {
      r.style.transform = "";
      r.style.transition = "";
    });
    if (motion) Flip.from(state, { duration: 0.35, ease: "power3.out", targets: rows() });
  }, [settings.sidebar]);

  const commit = (next: Block[]) => {
    settle.current = Flip.getState(rows());
    save({ ...settings, sidebar: next });
  };

  const move = (from: number, to: number) => {
    if (to < 0 || to >= order.length || to === from) return;
    const next = [...order];
    const [id] = next.splice(from, 1);
    next.splice(to, 0, id);
    commit(next);
  };

  const shift = (d: Drag, to: number) => {
    rows().forEach((r, i) => {
      if (i === d.from) return;
      let y = 0;
      if (d.from < to && i > d.from && i <= to) y = -d.step;
      if (d.from > to && i < d.from && i >= to) y = d.step;
      r.style.transition = "transform .22s cubic-bezier(.16, 1, .3, 1)";
      r.style.transform = y ? `translate3d(0, ${y}px, 0)` : "";
    });
  };

  const down = (e: PointerEvent<HTMLButtonElement>, id: Block, index: number) => {
    if (e.button !== 0) return;
    const r = rows();
    const step = r.length > 1 ? r[1].offsetTop - r[0].offsetTop : r[0]?.offsetHeight ?? 52;
    e.currentTarget.setPointerCapture(e.pointerId);
    setDrag({ id, from: index, to: index, startY: e.clientY, step });
  };

  const moveDrag = (e: PointerEvent<HTMLButtonElement>) => {
    if (!drag) return;
    const dy = e.clientY - drag.startY;
    const row = rows()[drag.from];
    if (row) {
      row.style.transition = "none";
      row.style.transform = `translate3d(0, ${dy}px, 0) scale(1.015)`;
    }
    const to = Math.max(0, Math.min(order.length - 1, Math.round(drag.from + dy / drag.step)));
    if (to !== drag.to) {
      shift(drag, to);
      setDrag({ ...drag, to });
    }
  };

  const up = () => {
    if (!drag) return;
    const d = drag;
    setDrag(null);
    if (d.to === d.from) {
      const state = Flip.getState(rows());
      rows().forEach((r) => {
        r.style.transform = "";
        r.style.transition = "";
      });
      if (motion) Flip.from(state, { duration: 0.3, ease: "power3.out" });
      return;
    }
    move(d.from, d.to);
  };

  const key = (e: KeyboardEvent<HTMLButtonElement>, index: number) => {
    if (e.key !== "ArrowUp" && e.key !== "ArrowDown") return;
    e.preventDefault();
    const to = index + (e.key === "ArrowUp" ? -1 : 1);
    move(index, to);
    requestAnimationFrame(() => rows()[Math.max(0, Math.min(order.length - 1, to))]?.querySelector<HTMLButtonElement>("button[data-grip]")?.focus());
  };

  const toggle = (id: Block, on: boolean) => {
    const hidden = new Set(settings.sidebarHidden ?? []);
    if (on) hidden.delete(id);
    else hidden.add(id);
    save({ ...settings, sidebarHidden: [...hidden] });
  };

  const reset = () => {
    settle.current = Flip.getState(rows());
    save({ ...settings, sidebar: [], sidebarHidden: [] });
  };
  const custom = (settings.sidebar?.length ?? 0) > 0 || (settings.sidebarHidden?.length ?? 0) > 0;

  return (
    <div className="flex flex-col gap-3 px-5 py-4">
      <div className="flex items-start gap-4">
        <div className="min-w-0 flex-1">
          <div className="text-[14.5px] font-semibold">Blocks</div>
          <div className="mt-0.5 text-[13px] leading-snug text-ink-3">Drag a block by its grip to move it, or focus the grip and use the arrow keys. Switch a block off to hide it. Settings always stays, so you can get back here.</div>
        </div>
        <button type="button" className="btn sm shrink-0" disabled={!custom} onClick={reset}>
          Reset
        </button>
      </div>
      <div ref={list} className="flex flex-col gap-1.5" role="list">
        {order.map((id, i) => {
          const hidden = blockHidden(settings.sidebarHidden, id);
          const dragging = drag?.id === id;
          return (
            <div
              key={id}
              data-block={id}
              role="listitem"
              className={`block-row ${dragging ? "dragging" : ""} ${hidden ? "off" : ""}`}
            >
              <button
                type="button"
                data-grip
                className="block-grip"
                aria-label={`Move ${BLOCK_LABEL[id]}, position ${i + 1} of ${order.length}`}
                onPointerDown={(e) => down(e, id, i)}
                onPointerMove={moveDrag}
                onPointerUp={up}
                onPointerCancel={up}
                onKeyDown={(e) => key(e, i)}
              >
                <Grip />
              </button>
              <span className="block-icon">{icons[id]}</span>
              <span className="min-w-0 flex-1 truncate text-[14px]">{BLOCK_LABEL[id]}</span>
              {id === ALWAYS_SHOWN ? (
                <span className="eyebrow">always on</span>
              ) : (
                <Toggle on={!hidden} onChange={(v) => toggle(id, v)} label={`Show ${BLOCK_LABEL[id]} in the sidebar`} />
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
