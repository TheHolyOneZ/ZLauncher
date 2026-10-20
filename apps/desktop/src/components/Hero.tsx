import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { canPlayVideo, Job, Library, OS_LABEL, osesOf, Project, summary } from "../api";
import { Action } from "./Action";
import { Icon } from "./Icon";
import { hideVideo, pauseVideo, resumeVideo, showVideo } from "./videoPool";

const ROTATE_MS = 9000;

export function Hero({ items, lib, jobs, open, active = true }: { items: Project[]; lib: Library; jobs: Record<string, Job>; open: (id: string) => void; active?: boolean }) {
  const [i, setI] = useState(0);
  const [paused, setPaused] = useState(false);
  const p = items[i % Math.max(items.length, 1)];

  const box = useRef<HTMLDivElement>(null);
  const root = useRef<HTMLElement>(null);
  const [inView, setInView] = useState(true);
  useEffect(() => {
    const el = root.current;
    if (!el) return;
    const io = new IntersectionObserver(([e]) => setInView(e.isIntersecting), { threshold: 0.05 });
    io.observe(el);
    return () => io.disconnect();
  }, []);
  const mp4 = p?.video && canPlayVideo ? p.video.mp4 : null;
  useLayoutEffect(() => {
    const el = box.current;
    if (!el || !mp4) {
      hideVideo("hero");
      return;
    }
    showVideo("hero", el, mp4);
  }, [mp4]);
  useEffect(() => () => pauseVideo("hero"), []);
  useEffect(() => {
    if (active && inView) resumeVideo("hero");
    else pauseVideo("hero");
  }, [active, inView, mp4]);

  useEffect(() => {
    if (paused || !active || items.length < 2) return;
    const t = window.setTimeout(() => setI((n) => (n + 1) % items.length), ROTATE_MS);
    return () => window.clearTimeout(t);
  }, [i, paused, active, items.length]);

  if (!p) return null;
  const oses = osesOf(p);

  return (
    <section
      ref={root}
      style={{ clipPath: "inset(0 round 20px)" }}
      className="relative isolate h-[380px] overflow-hidden rounded-[20px] bg-black shadow-[var(--shadow)]"
      onMouseEnter={() => setPaused(true)}
      onMouseLeave={() => setPaused(false)}
    >
      <div key={p.id} className="absolute inset-0 animate-[fadein_.8s_ease_both]">
        {p.video?.poster && <img src={p.video.poster} alt="" className="kenburns absolute inset-0 h-full w-full object-cover" />}
      </div>
      <div ref={box} className="absolute inset-0" />
      <div className="hero-shade absolute inset-0" />
      <div className="pointer-events-none absolute inset-0 z-10 rounded-[20px] shadow-[inset_0_0_0_1px_rgba(255,255,255,.07)]" />

      <div key={`t-${p.id}`} className="page relative flex h-full max-w-[560px] flex-col justify-end gap-3 p-9 text-white">
        <div className="flex items-center gap-2 text-[12px] font-semibold tracking-[.14em] text-white/60 uppercase">
          <span className="h-1.5 w-1.5 rounded-full bg-cyan shadow-[0_0_12px_var(--cyan)]" /> New on zsync · {p.category}
        </div>
        <h2 className="text-[48px] leading-[1] font-bold">{p.title}</h2>
        <p className="text-[15.5px] leading-relaxed text-white/75">{summary(p)}</p>
        <div className="flex items-center gap-2 text-[12px] text-white/55">
          {p.version && <span className="font-mono">v{p.version}</span>}
          {oses.length > 0 && <span>· {oses.map((o) => OS_LABEL[o]).join(" · ")}</span>}
        </div>
        <div className="mt-2 flex items-center gap-2.5">
          <Action p={p} lib={lib} job={jobs[p.id]} big />
          <button className="btn glass lg" onClick={() => open(p.id)}>
            Details <Icon name="back" className="rotate-180" />
          </button>
        </div>
      </div>

      {items.length > 1 && (
        <div className="absolute right-6 bottom-6 flex gap-1.5">
          {items.map((it, n) => (
            <button
              key={it.id}
              aria-label={it.title}
              className={`h-1.5 rounded-full transition-all duration-300 ${n === i ? "w-7 bg-white" : "w-1.5 bg-white/35 hover:bg-white/60"}`}
              onClick={() => setI(n)}
            />
          ))}
        </div>
      )}
    </section>
  );
}
