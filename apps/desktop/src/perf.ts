import { invoke } from "@tauri-apps/api/core";
import type { ProfilerOnRenderCallback } from "react";

const on = import.meta.env.DEV && "__TAURI_INTERNALS__" in window;
const FREEZE_MS = 80;
const SLOW_RENDER_MS = 25;

let lastAction = "start";
let lastActionAt = performance.now();

function send(line: string) {
  if (on) invoke("log", { message: `perf ${line}` }).catch(() => {});
}

export function devNote(line: string) {
  send(`note ${line}`);
}

export function timed<T>(label: string, f: () => T): T {
  if (!on) return f();
  const t = performance.now();
  const r = f();
  const ms = performance.now() - t;
  if (ms >= 15) send(`slow ${label} ${ms.toFixed(0)}ms  after "${lastAction}"`);
  return r;
}

export function untilPainted(label: string) {
  if (!on) return;
  const t = performance.now();
  requestAnimationFrame(() =>
    setTimeout(() => {
      const ms = performance.now() - t;
      if (ms >= 60) send(`paint ${label} ${ms.toFixed(0)}ms  after "${lastAction}"`);
    }, 0),
  );
}

export function markAction(what: string) {
  lastAction = what;
  lastActionAt = performance.now();
}

export const onRender: ProfilerOnRenderCallback = (id, phase, actual) => {
  if (actual >= SLOW_RENDER_MS) send(`render ${id} ${phase} ${actual.toFixed(0)}ms  after "${lastAction}"`);
};

if (on) {
  let prev = performance.now();
  const tick = (now: number) => {
    const gap = now - prev;
    if (gap > FREEZE_MS && document.visibilityState === "visible") {
      send(`freeze ${gap.toFixed(0)}ms  ${(now - lastActionAt - gap).toFixed(0)}ms after "${lastAction}"`);
    }
    prev = now;
    requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
  send("recorder on");
}
