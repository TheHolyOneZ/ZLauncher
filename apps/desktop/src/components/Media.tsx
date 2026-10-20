import { useLayoutEffect, useRef } from "react";
import { canPlayVideo, Project } from "../api";
import { hideVideo, showVideo } from "./videoPool";

export function Media({ p, playing = false, className = "" }: { p: Project; playing?: boolean; className?: string }) {
  const v = p.video;
  const box = useRef<HTMLDivElement>(null);

  useLayoutEffect(() => {
    const el = box.current;
    if (!playing || !v || !canPlayVideo || !el) return;
    showVideo("card", el, v.mp4);
    return () => hideVideo("card", el);
  }, [playing, v, v?.mp4]);

  return (
    <div ref={box} className={`media relative overflow-hidden bg-bg-2 ${className}`}>
      {v?.poster ? (
        <img className="h-full w-full object-cover" src={v.poster} alt="" loading="lazy" decoding="async" draggable={false} />
      ) : (
        <div className="tile grid h-full w-full place-items-center">
          <span className="font-head text-5xl font-bold text-ink/70">{p.title.replace(/^Z[-\s]?/, "").slice(0, 1) || "Z"}</span>
        </div>
      )}
    </div>
  );
}
