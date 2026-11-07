import { createContext, ReactNode, useContext, useEffect, useMemo, useState } from "react";
import { gsap } from "gsap";
import { useGSAP } from "@gsap/react";
import { Flip } from "gsap/Flip";

gsap.registerPlugin(useGSAP, Flip);

export type EffectsLevel = "full" | "calm" | "off";

export interface Palette {
  bg: string;
  cyan: string;
  violet: string;
  ink: string;
}

export interface Effects {
  level: EffectsLevel;
  motion: boolean;
  theme: "dark" | "light";
  palette: Palette;
}

const DARK: Palette = { bg: "#0b0e16", cyan: "#3fc6fb", violet: "#a664f9", ink: "#e9edf5" };

const Ctx = createContext<Effects>({ level: "calm", motion: false, theme: "dark", palette: DARK });

export const useEffectsLevel = () => useContext(Ctx);

function query(q: string): boolean {
  return typeof window !== "undefined" && !!window.matchMedia?.(q).matches;
}

function readTheme(): "dark" | "light" {
  const t = document.documentElement.dataset.theme;
  if (t === "dark" || t === "light") return t;
  return query("(prefers-color-scheme: light)") ? "light" : "dark";
}

function readPalette(): Palette {
  const css = getComputedStyle(document.documentElement);
  const v = (name: string, fallback: string) => css.getPropertyValue(name).trim() || fallback;
  return { bg: v("--bg", DARK.bg), cyan: v("--cyan", DARK.cyan), violet: v("--violet", DARK.violet), ink: v("--ink", DARK.ink) };
}

export function EffectsProvider({ level: wanted, children }: { level: EffectsLevel; children: ReactNode }) {
  const [reduced, setReduced] = useState(() => query("(prefers-reduced-motion: reduce)"));
  const [still] = useState(() => new URLSearchParams(location.hash.slice(1)).has("still"));
  const [theme, setTheme] = useState(readTheme);
  const [palette, setPalette] = useState(readPalette);

  useEffect(() => {
    const motionQuery = window.matchMedia("(prefers-reduced-motion: reduce)");
    const schemeQuery = window.matchMedia("(prefers-color-scheme: light)");
    const sync = () => {
      setTheme(readTheme());
      setPalette(readPalette());
    };
    const onMotion = () => setReduced(motionQuery.matches);
    const watch = new MutationObserver(sync);
    watch.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
    motionQuery.addEventListener("change", onMotion);
    schemeQuery.addEventListener("change", sync);
    return () => {
      watch.disconnect();
      motionQuery.removeEventListener("change", onMotion);
      schemeQuery.removeEventListener("change", sync);
    };
  }, []);

  const level: EffectsLevel = wanted === "full" && (reduced || still) ? "calm" : wanted;
  const value = useMemo<Effects>(() => ({ level, motion: level === "full", theme, palette }), [level, theme, palette]);

  useEffect(() => {
    document.documentElement.dataset.fx = level;
    if (import.meta.env.DEV && location.hash.includes("fxonly")) document.documentElement.classList.add("fxonly");
  }, [level]);

  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

export { gsap, Flip, useGSAP };
