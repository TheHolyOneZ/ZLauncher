import { useEffect, useRef, useState } from "react";
import { api, Job, Library, Missing, Project } from "../api";
import { isActive } from "../state";
import { Progress } from "./Action";
import { Icon } from "./Icon";
import { ask, fail } from "./Overlays";

function CopyLine({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <button
      className="selectable group flex w-full items-center gap-2 rounded-lg bg-bg-2 px-3 py-1.5 text-left font-mono text-[12px] text-ink-2 hover:text-ink"
      title="Copy"
      onClick={() => {
        navigator.clipboard?.writeText(text).then(() => {
          setCopied(true);
          window.setTimeout(() => setCopied(false), 1500);
        });
      }}
    >
      <span className="min-w-0 flex-1 truncate">{text}</span>
      <span className="shrink-0 text-[11px] text-ink-3 group-hover:text-cyan">{copied ? "copied" : "copy"}</span>
    </button>
  );
}

function MissingList({ missing }: { missing: Missing[] }) {
  return (
    <div className="flex flex-col gap-2.5">
      {missing.map((m) => (
        <div key={m.name} className="flex flex-col gap-1">
          <div className="text-[13px]">
            <span className="font-semibold">{m.name}</span> <span className="text-ink-3">· {m.why}</span>
          </div>
          <CopyLine text={m.install} />
        </div>
      ))}
    </div>
  );
}

function Log({ lines }: { lines: string[] }) {
  const ref = useRef<HTMLPreElement>(null);
  useEffect(() => {
    const el = ref.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [lines.length]);
  return (
    <pre ref={ref} className="selectable scroll max-h-64 overflow-x-auto rounded-xl bg-bg-2 p-3 font-mono text-[11.5px] leading-[1.6] text-ink-2">
      {lines.slice(-250).join("\n")}
    </pre>
  );
}

export function BuildPanel({ p, lib, job }: { p: Project; lib: Library; job?: Job }) {
  const [missing, setMissing] = useState<Missing[] | null>(null);
  const [showLog, setShowLog] = useState(false);
  const repo = p.links.find((l) => l.url.includes("github.com"))?.url.replace(/^https:\/\/github\.com\//, "").replace(/\/$/, "");
  const building = job?.phase === "building" || (isActive(job) && !!job?.builtFrom);
  const lastWasBuild = !!job && (job.phase === "building" || job.builtFrom !== null || job.missing.length > 0);

  useEffect(() => {
    api.buildRequirements().then(setMissing).catch(() => setMissing([]));
  }, [job?.phase]);

  const forThisOs = p.artifacts.length === 0 || p.artifacts.some((a) => a.os === lib.host.os);
  if (!repo || !forThisOs || lib.host.os === "android" || lib.host.os === "ios") return null;
  const jobMissing = lastWasBuild && job?.missing.length ? job.missing : null;
  const ready = missing !== null && missing.length === 0;

  return (
    <section className="card flex flex-col gap-4 p-5">
      <div className="flex items-start gap-4">
        <span className="grid h-10 w-10 shrink-0 place-items-center rounded-xl bg-hi text-cyan">
          <Icon name="code" />
        </span>
        <div className="min-w-0 flex-1">
          <h3 className="text-[17px] font-bold">Build it yourself</h3>
          <p className="text-[13.5px] text-ink-2">
            Compile {p.title} from its source on this computer, so it is made for exactly your system. The launcher clones{" "}
            <span className="font-mono text-[12.5px]">{repo}</span> at the release you see here and installs the result like a download.
            The first build takes a few minutes; updates are faster.
          </p>
        </div>
        {!isActive(job) && (
          <button
            className="btn shrink-0"
            disabled={missing === null || !ready}
            title={ready ? "Build on this PC" : "Some build tools are missing"}
            onClick={async () => (await ask({ title: `Build ${p.title} from source?`, body: "This downloads its source code and runs the project's own build scripts on this computer.", ok: "Build" })) && api.build(p.id).catch(fail)}
          >
            <Icon name="code" /> Build on this PC
          </button>
        )}
      </div>

      {missing && missing.length > 0 && !building && (
        <div className="flex flex-col gap-3 rounded-xl border border-mark/30 p-4">
          <div className="flex items-center gap-2 text-[13.5px] font-semibold">
            <Icon name="warn" className="text-mark" /> Install these first, then come back:
          </div>
          <MissingList missing={missing} />
        </div>
      )}
      {ready && !lastWasBuild && (
        <div className="flex items-center gap-2 text-[13px] text-ok">
          <Icon name="check" /> All build tools found on this computer.
        </div>
      )}

      {lastWasBuild && job && (
        <div className="flex flex-col gap-3">
          {isActive(job) && <Progress job={job} />}
          {job.builtFrom && <div className="font-mono text-[12px] text-ink-3">source: {repo} @ {job.builtFrom}</div>}
          {job.phase === "failed" && (
            <div className="selectable text-[13.5px] text-bad">
              {job.error}
              {job.hint && <div className="text-ink-3">{job.hint}</div>}
            </div>
          )}
          {jobMissing && <MissingList missing={jobMissing} />}
          {job.phase === "done" && (
            <div className="flex items-center gap-2 text-[13.5px] text-ok">
              <Icon name="check" /> Built on this computer and installed.
            </div>
          )}
          {job.log.length > 0 && (
            <>
              <button className="self-start text-[12.5px] text-ink-3 hover:text-ink" onClick={() => setShowLog(!showLog)}>
                {showLog || building ? "Hide" : "Show"} build log ({job.log.length} lines)
              </button>
              {(showLog || building) && <Log lines={job.log} />}
            </>
          )}
        </div>
      )}
    </section>
  );
}
