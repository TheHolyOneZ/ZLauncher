import { devNote, timed } from "../perf";

const PAUSE_AFTER_MS = 20_000;
const SAFE_AGE_MS = 15_000;
const GROUP_SIZE: Record<string, number> = { card: 6, hero: 6, detail: 2 };
const SPARE = 3;

interface Slot {
  v: HTMLVideoElement;
  since: number;
  used: number;
  pauseTimer?: number;
}

const groups = new Map<string, Slot[]>();
const shown = new Map<string, Slot>();
let garage: HTMLDivElement | null = null;

function garageEl(): HTMLDivElement {
  if (!garage) {
    garage = document.createElement("div");
    garage.setAttribute("aria-hidden", "true");
    Object.assign(garage.style, {
      position: "fixed",
      left: "0",
      top: "0",
      width: "1px",
      height: "1px",
      overflow: "hidden",
      opacity: "0",
      pointerEvents: "none",
      zIndex: "-1",
    });
    document.body.appendChild(garage);
  }
  return garage;
}

function make(): HTMLVideoElement {
  const v = document.createElement("video");
  v.muted = false;
  v.volume = 0;
  v.loop = true;
  v.playsInline = true;
  v.preload = "auto";
  v.setAttribute("aria-hidden", "true");
  Object.assign(v.style, {
    position: "absolute",
    inset: "0",
    width: "100%",
    height: "100%",
    objectFit: "cover",
    opacity: "0",
    transition: "opacity .5s ease",
    pointerEvents: "none",
  });
  v.addEventListener("playing", () => {
    if (v.parentElement !== garage) v.style.opacity = "1";
  });
  return v;
}

function slotFor(key: string, src: string): Slot | null {
  const list = groups.get(key) ?? [];
  groups.set(key, list);
  const have = list.find((s) => s.v.dataset.src === src);
  if (have) return have;

  const now = performance.now();
  const max = GROUP_SIZE[key] ?? 4;
  if (list.length >= max) {
    const free = list
      .filter((s) => s !== shown.get(key) && now - s.since >= SAFE_AGE_MS)
      .sort((a, b) => a.used - b.used)[0];
    if (free) {
      window.clearTimeout(free.pauseTimer);
      free.v.style.opacity = "0";
      free.v.dataset.src = src;
      free.v.src = src;
      free.since = now;
      return free;
    }
    if (list.length >= max + SPARE) return null;
  }
  const slot: Slot = { v: make(), since: now, used: now };
  slot.v.dataset.src = src;
  slot.v.src = src;
  list.push(slot);
  return slot;
}

function play(key: string, s: Slot, what: string) {
  window.clearTimeout(s.pauseTimer);
  s.pauseTimer = undefined;
  s.v.play().catch((e) => {
    if (!(e instanceof DOMException && e.name === "AbortError")) devNote(`video ${key} ${what} refused: ${e}`);
  });
}

function pauseLater(key: string, s: Slot) {
  window.clearTimeout(s.pauseTimer);
  s.pauseTimer = window.setTimeout(() => {
    s.pauseTimer = undefined;
    timed(`video ${key} deferred pause`, () => s.v.pause());
  }, PAUSE_AFTER_MS);
}

function park(key: string, s: Slot) {
  s.v.style.opacity = "0";
  s.v.volume = 0;
  if (s.v.parentElement !== garage) garageEl().appendChild(s.v);
  if (shown.get(key) === s) shown.delete(key);
  pauseLater(key, s);
}

export interface VideoOptions {
  controls?: boolean;
}

export function showVideo(key: string, parent: HTMLElement, src: string, opts: VideoOptions = {}) {
  timed(`video ${key} show`, () => {
    const s = slotFor(key, src);
    const before = shown.get(key);
    if (before && before !== s) park(key, before);
    if (!s) return;
    const v = s.v;
    v.controls = !!opts.controls;
    v.muted = false;
    v.volume = opts.controls ? 1 : 0;
    v.style.pointerEvents = opts.controls ? "auto" : "none";
    if (!v.paused && v.readyState >= 2) v.style.opacity = "1";
    if (v.parentElement !== parent) parent.appendChild(v);
    s.used = performance.now();
    shown.set(key, s);
    play(key, s, "play");
  });
}

export function hideVideo(key: string, parent?: HTMLElement | null) {
  timed(`video ${key} hide`, () => {
    const s = shown.get(key);
    if (!s) return;
    if (parent && s.v.parentElement !== parent) return;
    park(key, s);
  });
}

export function pauseVideo(key: string) {
  const s = shown.get(key);
  if (s) pauseLater(key, s);
}

export function resumeVideo(key: string) {
  const s = shown.get(key);
  if (s && s.v.parentElement !== garage) {
    if (!s.v.paused) s.v.style.opacity = "1";
    play(key, s, "resume");
  }
}
