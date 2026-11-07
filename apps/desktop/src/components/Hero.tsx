import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { canPlayVideo, Job, Library, OS_LABEL, osesOf, Project, summary } from "../api";
import { Action } from "./Action";
import { Icon } from "./Icon";
import { hideVideo, pauseVideo, resumeVideo, showVideo } from "./videoPool";
import { gsap, useEffectsLevel, useGSAP } from "../effects";
import SplitText from "./bits/SplitText/SplitText";
import Magnet from "./bits/Magnet/Magnet";

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

  const { motion } = useEffectsLevel();
  const depth = useRef<HTMLDivElement>(null);
  useGSAP(
    () => {
      const el = root.current;
      const layer = depth.current;
      if (!motion || !el || !layer) return;
      const x = gsap.quickTo(layer, "x", { duration: 1.1, ease: "power3.out" });
      const y = gsap.quickTo(layer, "y", { duration: 1.1, ease: "power3.out" });
      const move = (e: PointerEvent) => {
        const r = el.getBoundingClientRect();
        x(((e.clientX - r.left) / r.width - 0.5) * -22);
        y(((e.clientY - r.top) / r.height - 0.5) * -14);
      };
      const leave = () => {
        x(0);
        y(0);
      };
      el.addEventListener("pointermove", move);
      el.addEventListener("pointerleave", leave);
      return () => {
        el.removeEventListener("pointermove", move);
        el.removeEventListener("pointerleave", leave);
        gsap.set(layer, { clearProps: "transform" });
      };
    },
    { dependencies: [motion] },
  );
  const copy = useRef<HTMLDivElement>(null);
  useGSAP(
    () => {
      if (!motion) return;
      gsap.from(".hero-in", { y: 18, opacity: 0, duration: 0.8, ease: "expo.out", stagger: 0.07, delay: 0.12, clearProps: "transform,opacity" });
    },
    { scope: copy, dependencies: [p?.id, motion] },
  );

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
      <div ref={depth} className="absolute -inset-4">
        <div key={p.id} className="absolute inset-0 animate-[fadein_.8s_ease_both]">
          {p.video?.poster && <img src={p.video.poster} alt="" className="kenburns absolute inset-0 h-full w-full object-cover" />}
        </div>
        <div ref={box} className="absolute inset-0" />
      </div>
      <div className="hero-shade absolute inset-0" />
      <div className="pointer-events-none absolute inset-0 z-10 rounded-[20px] shadow-[inset_0_0_0_1px_rgba(255,255,255,.07)]" />

      <div ref={copy} key={`t-${p.id}`} className={`${motion ? "" : "page"} relative flex h-full max-w-[560px] flex-col justify-end gap-3 p-9 text-white`}>
        <div className="hero-in eyebrow flex items-center gap-2 !text-white/60">
          <span className="h-1.5 w-1.5 rounded-full bg-[#19e3ff] shadow-[0_0_12px_#19e3ff]" /> <b className="!text-[#19e3ff]">New on zsync</b> · {p.category}
        </div>
        {motion ? (
          <SplitText
            key={p.id}
            text={p.title}
            tag="h2"
            className="hero-title display !overflow-visible pb-1 text-[60px] leading-[1]"
            splitType="chars"
            effect="rise"
            trigger="load"
            delay={26}
            duration={0.95}
            textAlign="left"
            hoverEffect="wave"
          />
        ) : (
          <h2 className="display text-[60px] leading-[1]">{p.title}</h2>
        )}
        <p className="hero-in text-[15.5px] leading-relaxed text-white/75">{summary(p)}</p>
        <div className="hero-in flex items-center gap-2 text-[12px] text-white/55">
          {p.version && <span className="font-mono">v{p.version}</span>}
          {oses.length > 0 && <span>· {oses.map((o) => OS_LABEL[o]).join(" · ")}</span>}
        </div>
        <div className="hero-in mt-2 flex items-center gap-2.5">
          <Magnet padding={70} magnetStrength={5} maxOffset={9} disabled={!motion}>
            <Action p={p} lib={lib} job={jobs[p.id]} big />
          </Magnet>
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
