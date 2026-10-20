import { useState } from "react";
import { api, bytes, eta, Job, Library, rate } from "../api";
import { Progress } from "../components/Action";
import { Icon } from "../components/Icon";
import { Media } from "../components/Media";
import { SpeedGraph } from "../components/SpeedGraph";
import { isActive, SpeedHistory } from "../state";
import { fail, menu, MenuItem } from "../components/Overlays";
import { copyText } from "../components/projectMenu";

const PHASE: Record<Job["phase"], string> = {
  queued: "Waiting",
  building: "Building from source",
  downloading: "Downloading",
  installing: "Installing",
  removing: "Removing",
  done: "Done",
  failed: "Failed",
  cancelled: "Stopped",
};

function Stat({ label, value, big = false }: { label: string; value: string; big?: boolean }) {
  return (
    <div className="flex flex-col gap-0.5">
      <span className="text-[11px] font-semibold tracking-[.12em] text-ink-3 uppercase">{label}</span>
      <span className={`font-head font-semibold text-ink tabular-nums ${big ? "text-[34px] leading-none tracking-tight" : "text-[17px]"}`}>{value}</span>
    </div>
  );
}

function LogView({ lines }: { lines: string[] }) {
  return (
    <pre className="selectable scroll max-h-56 overflow-x-auto rounded-xl bg-bg-2 p-3 font-mono text-[11.5px] leading-[1.6] text-ink-2">
      {lines.slice(-200).join("\n")}
    </pre>
  );
}

function Current({ j, lib, history }: { j: Job; lib: Library; history?: SpeedHistory }) {
  const [log, setLog] = useState(false);
  const p = lib.projects.find((x) => x.id === j.projectId);
  const downloading = j.phase === "downloading";
  const pct = j.total ? Math.min(100, (j.done / j.total) * 100) : 0;
  const left = downloading ? eta(j.done, j.total, j.speed, false) : null;
  return (
    <section className="card relative overflow-hidden p-6">
      <div className="glow -top-36 -left-36 h-[26rem] w-[26rem] [--glow:color-mix(in_srgb,var(--cyan)_14%,transparent)]" />
      <div className="relative flex items-start gap-5">
        {p && (
          <div className="w-44 shrink-0 overflow-hidden rounded-xl border border-line">
            <Media p={p} className="aspect-video" />
          </div>
        )}
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2 text-[12px] font-semibold tracking-[.12em] text-cyan uppercase">
            <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-cyan" />
            {PHASE[j.phase]}
          </div>
          <h2 className="mt-1 truncate text-[26px] font-bold">{j.title}</h2>
          <div className="truncate font-mono text-[12px] text-ink-3">{j.filename}</div>
        </div>
        {(downloading || j.phase === "building") && (
          <button className="btn shrink-0" onClick={() => api.cancel(j.projectId)}>
            <Icon name="x" /> Stop
          </button>
        )}
      </div>

      {downloading ? (
        <div className="relative mt-6 flex flex-col gap-5">
          <div className="flex flex-wrap items-end gap-x-10 gap-y-4">
            <Stat label="Speed" value={rate(j.speed)} big />
            <Stat label="Downloaded" value={`${bytes(j.done) || "0 B"}${j.total ? ` of ${bytes(j.total)}` : ""}`} />
            <Stat label="Time left" value={left ?? "…"} />
            {history && history.peak > 0 && <Stat label="Peak" value={rate(history.peak)} />}
            <span className="ml-auto font-head text-[40px] leading-none font-bold text-ink-3 tabular-nums">{Math.floor(pct)}%</span>
          </div>
          <div className="bar !h-2">
            <i style={{ width: `${pct}%` }} />
          </div>
          <SpeedGraph history={history} height={150} />
        </div>
      ) : (
        <div className="relative mt-6 flex flex-col gap-3">
          <Progress job={j} />
          {j.log.length > 0 && (
            <>
              <button className="self-start text-[12.5px] text-ink-3 hover:text-ink" onClick={() => setLog(!log)}>
                {log ? "Hide" : "Show"} details ({j.log.length} lines)
              </button>
              {log && <LogView lines={j.log} />}
            </>
          )}
        </div>
      )}
    </section>
  );
}

function Running({ j, history }: { j: Job; history?: SpeedHistory }) {
  const pct = j.total ? Math.min(100, (j.done / j.total) * 100) : 0;
  return (
    <div className="card flex items-center gap-4 p-4">
      <div className="min-w-0 flex-1">
        <div className="flex items-baseline gap-2">
          <span className="truncate font-semibold">{j.title}</span>
          <span className="shrink-0 text-[12px] text-ink-3">{PHASE[j.phase]}</span>
          {j.phase !== "downloading" && j.step && <span className="truncate text-[12px] text-ink-3">· {j.step}</span>}
        </div>
        <div className={`mt-2 bar ${j.phase === "downloading" && j.total ? "" : "indet"}`}>
          <i style={{ width: `${pct}%` }} />
        </div>
      </div>
      {j.phase === "downloading" && (
        <>
          <div className="w-32">
            <SpeedGraph history={history} height={34} compact />
          </div>
          <div className="w-24 text-right font-mono text-[12.5px] text-ink-2">
            {rate(j.speed)}
            <div className="text-[11px] text-ink-3">{eta(j.done, j.total, j.speed) ?? ""}</div>
          </div>
        </>
      )}
      <button className="btn sm icon" title="Stop" onClick={() => api.cancel(j.projectId)}>
        <Icon name="x" />
      </button>
    </div>
  );
}

function Finished({ j, clear, open }: { j: Job; clear: () => void; open: (id: string) => void }) {
  const [log, setLog] = useState(false);
  const failed = j.phase === "failed";
  const items: MenuItem[] = [
    { label: "Show tool", icon: "info", run: () => open(j.projectId) },
    ...(j.savedTo ? [{ label: "Show in folder", icon: "folder", run: () => api.reveal(j.savedTo!).catch(fail) }, { label: "Copy file path", icon: "copy", run: () => copyText(j.savedTo!) }] : []),
    ...(j.log.length ? [{ label: "Copy details", icon: "list", run: () => copyText(j.log.join("\n")) }] : []),
    "-" as const,
    { label: "Remove from the list", icon: "x", run: clear },
  ];
  return (
    <div className="flex flex-col gap-2 px-4 py-3" onContextMenu={(e) => menu(e, items)}>
      <div className="flex items-center gap-3">
        <span
          className={`grid h-8 w-8 shrink-0 place-items-center rounded-full ${failed ? "bg-bad/15 text-bad" : j.phase === "done" ? "bg-ok/15 text-ok" : "bg-bg-2 text-ink-3"}`}
        >
          <Icon name={failed ? "warn" : j.phase === "done" ? "check" : "x"} className="!h-4 !w-4" />
        </span>
        <div className="min-w-0 flex-1">
          <div className="truncate font-semibold">{j.title}</div>
          <div className="truncate text-[12.5px] text-ink-3">
            {failed ? j.error : j.savedTo ? `Saved to ${j.savedTo}` : j.phase === "done" ? (j.step ?? "Installed") : "Stopped. Starting again resumes it."}
          </div>
        </div>
        {j.savedTo && (
          <button className="btn sm" onClick={() => api.reveal(j.savedTo!)}>
            <Icon name="folder" /> Show
          </button>
        )}
        {j.log.length > 0 && (
          <button className="btn sm" onClick={() => setLog(!log)}>
            {log ? "Hide log" : "Log"}
          </button>
        )}
        <button className="btn sm icon" title="Remove from the list" onClick={clear}>
          <Icon name="x" />
        </button>
      </div>
      {failed && j.hint && <div className="pl-11 text-[12.5px] text-ink-3">{j.hint}</div>}
      {log && <LogView lines={j.log} />}
    </div>
  );
}

export function Downloads({
  jobs,
  history,
  lib,
  clear,
  open,
}: {
  jobs: Record<string, Job>;
  history: Record<string, SpeedHistory>;
  lib: Library;
  clear: (id: string) => void;
  open: (id: string) => void;
}) {
  const list = Object.values(jobs);
  const active = list.filter(isActive);
  const current = active.find((j) => j.phase === "downloading") ?? active[0];
  const others = active.filter((j) => j !== current);
  const done = list.filter((j) => !isActive(j));
  const total = active.filter((j) => j.phase === "downloading").reduce((n, j) => n + j.speed, 0);

  if (list.length === 0) {
    return (
      <div className="page flex flex-col items-center gap-4 py-32 text-center">
        <span className="grid h-16 w-16 place-items-center rounded-2xl bg-hi text-cyan">
          <Icon name="download" className="!h-7 !w-7" />
        </span>
        <div>
          <div className="text-[18px] font-bold">Nothing downloading</div>
          <p className="text-[14px] text-ink-3">Installs and builds show up here while they run, with their speed.</p>
        </div>
      </div>
    );
  }

  return (
    <div className="page mx-auto flex max-w-[1080px] flex-col gap-5 px-8 py-7">
      <div className="flex items-baseline gap-3">
        <h1 className="text-[28px] font-bold">Downloads</h1>
        {active.length > 0 && (
          <span className="text-[14px] text-ink-3">
            {active.length} running{total > 0 ? ` · ${rate(total)}` : ""}
          </span>
        )}
      </div>
      {current && <Current j={current} lib={lib} history={history[current.projectId]} />}
      {others.length > 0 && (
        <div className="flex flex-col gap-2.5">
          {others.map((j) => (
            <Running key={j.projectId} j={j} history={history[j.projectId]} />
          ))}
        </div>
      )}
      {done.length > 0 && (
        <section className="flex flex-col gap-2">
          <div className="flex items-center justify-between px-1">
            <h2 className="text-[12px] font-semibold tracking-[.14em] text-ink-3 uppercase">Finished</h2>
            <button className="text-[12.5px] text-ink-3 hover:text-ink" onClick={() => done.forEach((j) => clear(j.projectId))}>
              Clear all
            </button>
          </div>
          <div className="card divide-y divide-line">
            {done.map((j) => (
              <Finished key={j.projectId} j={j} clear={() => clear(j.projectId)} open={open} />
            ))}
          </div>
        </section>
      )}
    </div>
  );
}
