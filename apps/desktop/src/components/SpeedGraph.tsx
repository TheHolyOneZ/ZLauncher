import { useId } from "react";
import { rate } from "../api";
import type { SpeedHistory } from "../state";

function smooth(pts: [number, number][]): string {
  if (pts.length < 2) return "";
  let d = `M${pts[0][0]},${pts[0][1]}`;
  for (let i = 0; i < pts.length - 1; i++) {
    const [x0, y0] = pts[i - 1] ?? pts[i];
    const [x1, y1] = pts[i];
    const [x2, y2] = pts[i + 1];
    const [x3, y3] = pts[i + 2] ?? pts[i + 1];
    const c1x = x1 + (x2 - x0) / 6;
    const c1y = y1 + (y2 - y0) / 6;
    const c2x = x2 - (x3 - x1) / 6;
    const c2y = y2 - (y3 - y1) / 6;
    d += ` C${c1x},${c1y} ${c2x},${c2y} ${x2},${y2}`;
  }
  return d;
}

export function SpeedGraph({ history, height = 120, compact = false }: { history?: SpeedHistory; height?: number; compact?: boolean }) {
  const id = useId().replace(/:/g, "");
  const W = 600;
  const H = height;
  const samples = history?.samples ?? [];
  if (samples.length < 2) {
    return <div className="skeleton w-full rounded-xl opacity-40" style={{ height }} />;
  }
  const smoothed = samples.map((_, i) => {
    const w = samples.slice(Math.max(0, i - 2), i + 1);
    return w.reduce((a, b) => a + b, 0) / w.length;
  });
  const top = Math.max(...smoothed, 1) * 1.18;
  const pad = compact ? 2 : 6;
  const pts: [number, number][] = smoothed.map((v, i) => [(i / (smoothed.length - 1)) * W, H - pad - (v / top) * (H - pad * 2)]);
  const line = smooth(pts);
  const area = `${line} L${W},${H} L0,${H} Z`;
  const peakAt = smoothed.indexOf(Math.max(...smoothed));
  const [px, py] = pts[peakAt];
  const [lx, ly] = pts[pts.length - 1];

  return (
    <div className="relative w-full" style={{ height }}>
      <svg viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none" className="absolute inset-0 h-full w-full overflow-visible">
        <defs>
          <linearGradient id={`fill-${id}`} x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor="var(--cyan)" stopOpacity=".38" />
            <stop offset="100%" stopColor="var(--violet)" stopOpacity="0" />
          </linearGradient>
          <linearGradient id={`line-${id}`} x1="0" y1="0" x2="1" y2="0">
            <stop offset="0%" stopColor="var(--violet)" />
            <stop offset="100%" stopColor="var(--cyan)" />
          </linearGradient>
        </defs>
        {!compact &&
          [0.25, 0.5, 0.75].map((f) => <line key={f} x1="0" x2={W} y1={H * f} y2={H * f} stroke="var(--line)" strokeDasharray="3 5" vectorEffect="non-scaling-stroke" />)}
        <path d={area} fill={`url(#fill-${id})`} />
        <path d={line} fill="none" stroke={`url(#line-${id})`} strokeWidth={compact ? 1.5 : 2.25} vectorEffect="non-scaling-stroke" strokeLinejoin="round" />
      </svg>
      <span className="absolute h-2.5 w-2.5 -translate-x-1/2 -translate-y-1/2 rounded-full bg-cyan shadow-[0_0_0_4px_var(--hi),0_0_14px_var(--cyan)]" style={{ left: `${(lx / W) * 100}%`, top: `${(ly / H) * 100}%` }} />
      {!compact && (
        <>
          <span className="absolute h-1.5 w-1.5 -translate-x-1/2 -translate-y-1/2 rounded-full bg-ink-2" style={{ left: `${(px / W) * 100}%`, top: `${(py / H) * 100}%` }} />
          <span
            className="absolute -translate-x-1/2 -translate-y-[170%] rounded-md border border-line-2 bg-card px-1.5 py-0.5 font-mono text-[10.5px] whitespace-nowrap text-ink-2"
            style={{ left: `${Math.min(92, Math.max(8, (px / W) * 100))}%`, top: `${(py / H) * 100}%` }}
          >
            peak {rate(history!.peak)}
          </span>
        </>
      )}
    </div>
  );
}
