import { api, bytes, eta, installable, Job, Library, Project, rate } from "../api";
import { isActive } from "../state";
import { Icon } from "./Icon";
import { fail } from "./Overlays";

export function Progress({ job }: { job: Job }) {
  const pct = job.total ? Math.min(100, (job.done / job.total) * 100) : 0;
  const indet = job.phase !== "downloading" || !job.total;
  const label =
    job.phase === "downloading"
      ? [`${bytes(job.done) || "0 B"}${job.total ? ` / ${bytes(job.total)}` : ""}`, job.speed ? rate(job.speed) : "", eta(job.done, job.total, job.speed) ?? ""]
          .filter(Boolean)
          .join(" · ")
      : job.phase === "building"
        ? `${job.step ?? "Building"}${job.log.length ? ` · ${job.log[job.log.length - 1]}` : ""}`
        : job.step ?? (job.phase === "removing" ? "Removing…" : job.phase === "queued" ? "Waiting…" : "Installing…");
  return (
    <div className="flex min-w-0 flex-1 flex-col gap-1.5">
      <div className={`bar ${indet ? "indet" : ""}`}>
        <i style={{ width: `${pct}%` }} />
      </div>
      <div className="truncate font-mono text-[11.5px] text-ink-3">
        {job.needsAuth && job.phase !== "downloading" ? "Waiting for your password… · " : ""}
        {label}
      </div>
    </div>
  );
}

export function Action({ p, lib, job, compact = false, big = false }: { p: Project; lib: Library; job?: Job; compact?: boolean; big?: boolean }) {
  const stop = (e: React.MouseEvent) => e.stopPropagation();
  const sm = compact ? "sm" : big ? "lg" : "";

  if (isActive(job)) {
    return (
      <div className="flex min-w-0 flex-1 items-center gap-2" onClick={stop}>
        <Progress job={job!} />
        {(job!.phase === "downloading" || job!.phase === "building") && (
          <button className={`btn icon ${sm}`} title="Stop" onClick={() => api.cancel(p.id)}>
            <Icon name="x" />
          </button>
        )}
      </div>
    );
  }

  const rec = p.artifacts.find((a) => a.url === p.recommended);
  const inst = p.installed;
  if (inst?.present && inst.update) {
    return (
      <button className={`btn upd ${sm}`} onClick={(e) => (stop(e), api.install(p.id))}>
        <Icon name="up" /> Update to {inst.update}
      </button>
    );
  }
  if (inst?.present) {
    return (
      <button className={`btn ${sm}`} onClick={(e) => (stop(e), api.launch(p.id).catch(fail))}>
        <Icon name="play" /> Open
      </button>
    );
  }
  if (rec) {
    return (
      <button className={`btn go ${sm}`} onClick={(e) => (stop(e), api.install(p.id).catch(fail))} title={rec.filename}>
        <Icon name="download" /> Install{!compact && rec.size ? ` · ${bytes(rec.size)}` : ""}
      </button>
    );
  }
  const here = p.artifacts.find((a) => a.os === lib.host.os && !installable(a.kind, lib.host.os));
  if (here) {
    return (
      <button className={`btn ${sm}`} onClick={(e) => (stop(e), api.install(p.id, here.url).catch(fail))} title={here.filename}>
        <Icon name="download" /> Download
      </button>
    );
  }
  return p.url ? (
    <button className={`btn ${sm}`} onClick={(e) => (stop(e), api.openUrl(p.url!))}>
      <Icon name="link" /> Website
    </button>
  ) : null;
}
