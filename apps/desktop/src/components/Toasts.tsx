import { useEffect, useRef, useState } from "react";
import { Job, Phase } from "../api";
import { Icon } from "./Icon";

const SHOW_MS = 6000;

interface Toast {
  key: string;
  job: Job;
}

function message(j: Job): string {
  if (j.phase === "failed") return j.error ?? "Something went wrong.";
  if (j.savedTo) return "Downloaded. It can't be installed here, so it's in your Downloads folder.";
  if (j.step === "Removed") return "Removed.";
  return "Installed and ready.";
}

export function Toasts({ jobs, open, onDismiss }: { jobs: Record<string, Job>; open: (id: string) => void; onDismiss: (j: Job) => void }) {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const seen = useRef<Record<string, Phase>>({});

  useEffect(() => {
    for (const j of Object.values(jobs)) {
      const before = seen.current[j.projectId];
      seen.current[j.projectId] = j.phase;
      const finished = j.phase === "done" || j.phase === "failed";
      if (finished && before && before !== j.phase) {
        const key = `${j.projectId}-${Date.now()}`;
        setToasts((t) => [...t.slice(-2), { key, job: j }]);
        window.setTimeout(() => setToasts((t) => t.filter((x) => x.key !== key)), SHOW_MS);
      }
    }
  }, [jobs]);

  return (
    <div className="pointer-events-none fixed right-5 bottom-5 z-50 flex w-[340px] flex-col gap-2.5">
      {toasts.map(({ key, job }) => (
        <div key={key} className="toast pointer-events-auto flex items-start gap-3 rounded-2xl p-3.5">
          <span className={`grid h-8 w-8 shrink-0 place-items-center rounded-full ${job.phase === "failed" ? "bg-bad/15 text-bad" : "bg-ok/15 text-ok"}`}>
            <Icon name={job.phase === "failed" ? "warn" : "check"} />
          </span>
          <button className="min-w-0 flex-1 text-left" onClick={() => open(job.projectId)}>
            <div className="font-semibold">{job.title}</div>
            <div className="line-clamp-2 text-[12.5px] text-ink-2">{message(job)}</div>
          </button>
          <button
            className="text-ink-3 hover:text-ink"
            title="Dismiss"
            onClick={() => {
              setToasts((t) => t.filter((x) => x.key !== key));
              if (job.phase === "done") onDismiss(job);
            }}
          >
            <Icon name="x" />
          </button>
        </div>
      ))}
    </div>
  );
}
