import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { api, ago, Artifact, bytes, canPlayVideo, installable, Job, kindLabel, Library, Os, OS_LABEL, osesOf, Project } from "../api";
import { Action } from "../components/Action";
import { Toggle } from "../components/Controls";
import { Icon } from "../components/Icon";
import { BuildPanel } from "../components/BuildPanel";
import { WhatsNew } from "../components/WhatsNew";
import { About, sentences } from "../components/About";
import { Heart } from "./Browse";
import { OsIcon } from "../components/OsIcon";
import { hideVideo, showVideo } from "../components/videoPool";
import { timed, untilPainted } from "../perf";
import { isActive } from "../state";
import { fail, menu } from "../components/Overlays";
import { projectMenu, uninstall } from "../components/projectMenu";

function FileRow({ a, p, lib, busy }: { a: Artifact; p: Project; lib: Library; busy: boolean }) {
  const canInstall = a.os === lib.host.os && installable(a.kind, lib.host.os);
  const recommended = a.url === p.recommended;
  return (
    <div className={`flex items-center gap-3 rounded-xl px-3.5 py-3 transition-colors ${recommended ? "bg-hi ring-1 ring-cyan/30" : "hover:bg-bg-2"}`}>
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-1.5">
          <span className="font-semibold">{kindLabel(a.kind)}</span>
          {a.arch && <span className="tag">{a.arch}</span>}
          {recommended && <span className="tag here">Best match</span>}
          {a.sha256 && (
            <span className="tag" title={`SHA-256 ${a.sha256}\nChecked after the download.`}>
              <Icon name="shield" className="!h-3 !w-3" /> verified
            </span>
          )}
        </div>
        <div className="selectable mt-0.5 truncate font-mono text-[11.5px] text-ink-3" title={a.url}>
          {a.filename}
          {a.size ? ` · ${bytes(a.size)}` : ""}
        </div>
      </div>
      <button
        className={`btn sm ${recommended && !p.installed?.present ? "go" : ""}`}
        disabled={busy}
        onClick={() => api.install(p.id, a.url).catch(fail)}
      >
        <Icon name="download" />
        {canInstall ? (p.installed?.present ? "Reinstall" : "Install") : "Download"}
      </button>
    </div>
  );
}

function Player({ p }: { p: Project }) {
  const [play, setPlay] = useState(false);
  const box = useRef<HTMLDivElement>(null);
  const v = p.video;
  useLayoutEffect(() => {
    const el = box.current;
    if (!play || !v || !canPlayVideo || !el) return;
    showVideo("detail", el, v.mp4, { controls: true });
    return () => hideVideo("detail", el);
  }, [play, v]);

  if (!v) {
    return (
      <div className="tile grid aspect-video place-items-center">
        <span className="font-head text-6xl font-bold opacity-80">{p.title}</span>
      </div>
    );
  }
  return (
    <div className="relative aspect-video w-full overflow-hidden bg-black">
      <button className="group absolute inset-0 block" onClick={() => canPlayVideo && setPlay(true)} disabled={play}>
        {v.poster && <img src={v.poster} alt="" className="h-full w-full object-cover transition-transform duration-700 group-hover:scale-[1.03]" decoding="async" />}
        {!play && <div className="absolute inset-0 bg-black/20 transition-colors group-hover:bg-black/10" />}
        {!play &&
          (canPlayVideo ? (
            <span className="absolute inset-0 m-auto grid h-16 w-16 place-items-center rounded-full bg-white/90 text-black shadow-2xl transition-transform group-hover:scale-110">
              <Icon name="play" className="!h-6 !w-6 translate-x-0.5 fill-current" />
            </span>
          ) : (
            <span className="absolute right-3 bottom-3 rounded-lg bg-black/75 px-2.5 py-1 text-[12px] text-white/80">
              Video needs GStreamer plugins (see Settings)
            </span>
          ))}
        {!play && v.duration && (
          <span className="absolute top-3 right-3 rounded-md bg-black/75 px-2 py-0.5 font-mono text-[11px] text-white/85">
            {Math.floor(v.duration / 60)}:{String(v.duration % 60).padStart(2, "0")}
          </span>
        )}
      </button>
      <div ref={box} className={play ? "absolute inset-0" : "hidden"} />
    </div>
  );
}

function PosterGlow({ src }: { src: string }) {
  const ref = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    const img = new Image();
    let alive = true;
    img.decoding = "async";
    img.onload = () => {
      const c = ref.current;
      const g = c?.getContext("2d");
      if (!alive || !c || !g) return;
      timed("backdrop draw", () => g.drawImage(img, 0, 0, c.width, c.height));
      c.style.opacity = "0.42";
    };
    img.src = src;
    return () => {
      alive = false;
    };
  }, [src]);
  return (
    <canvas
      ref={ref}
      width={24}
      height={14}
      className="absolute -inset-x-[10%] top-[-12%] h-[124%] w-[120%] opacity-0 transition-opacity duration-500"
      aria-hidden
    />
  );
}

export function ProjectPage({
  p,
  lib,
  job,
  back,
  clearJob,
  fav,
  toggleFav,
}: {
  p: Project;
  lib: Library;
  job?: Job;
  back: () => void;
  clearJob: (id: string) => void;
  fav: boolean;
  toggleFav: () => void;
}) {
  const busy = isActive(job);
  const byOs = new Map<Os, Artifact[]>();
  for (const a of p.artifacts) byOs.set(a.os, [...(byOs.get(a.os) ?? []), a]);
  const oses = osesOf(p).sort((a, b) => Number(b === lib.host.os) - Number(a === lib.host.os));
  const inst = p.installed;
  const source = p.links.find((l) => l.url.includes("github.com"));
  const rec = p.artifacts.find((a) => a.url === p.recommended);
  const lead = sentences(p.description)[0] ?? "";
  const [rest, setRest] = useState(false);
  useEffect(() => untilPainted(`detail ${p.id}`), [p.id]);
  useEffect(() => {
    let b = 0;
    const a = requestAnimationFrame(() => {
      b = requestAnimationFrame(() => setRest(true));
    });
    return () => {
      cancelAnimationFrame(a);
      cancelAnimationFrame(b);
    };
  }, []);

  return (
    <div className="page relative min-h-full" onContextMenu={(e) => menu(e, projectMenu(p, lib, job, { fav, toggleFav }))}>
      <div className="pointer-events-none absolute inset-x-0 top-0 h-[520px] overflow-hidden">
        {p.video?.poster && <PosterGlow src={p.video.poster} />}
        <div className="absolute inset-0 bg-gradient-to-b from-transparent via-bg/60 to-bg" />
      </div>

      <div className="relative mx-auto flex max-w-[1160px] flex-col gap-8 px-8 py-6">
        <button className="btn sm self-start" onClick={back} title="Back (Esc)">
          <Icon name="back" /> Back
        </button>

        <div className="grid items-start gap-8 lg:grid-cols-[1.4fr_1fr]">
          <div className="overflow-hidden rounded-[18px] border border-line shadow-[var(--shadow)]">
            <Player p={p} />
          </div>

          <div className="flex flex-col gap-4 pt-1">
            <div className="text-[12px] font-semibold tracking-[.14em] text-ink-3 uppercase">{p.category}</div>
            <h1 className="text-[44px] leading-[1.02] font-bold">{p.title}</h1>
            <p className="selectable max-w-[46ch] text-[16px] leading-[1.55] text-ink-2">{lead}</p>
            <div className="flex flex-wrap items-center gap-2.5 text-[13px] text-ink-2">
              {p.version && <span className="font-mono">v{p.version}</span>}
              {oses.map((o) => (
                <span key={o} className="flex items-center gap-1">
                  <OsIcon os={o} here={o === lib.host.os} size={13} /> {OS_LABEL[o]}
                </span>
              ))}
              {p.retired && <span className="tag">Retired</span>}
            </div>

            <div className="flex flex-wrap items-center gap-2 pt-1">
              <Action p={p} lib={lib} job={job} big />
              {p.url && (rec || p.installed?.present || p.artifacts.some((a) => a.os === lib.host.os)) && (
                <button className="btn lg icon" title="Website" onClick={() => api.openUrl(p.url!)}>
                  <Icon name="link" />
                </button>
              )}
              {source && (
                <button className="btn lg icon" title="Source code" onClick={() => api.openUrl(source.url)}>
                  <Icon name="code" />
                </button>
              )}
              <span className="btn lg icon !p-0">
                <Heart on={fav} toggle={toggleFav} className="!h-full !w-full" />
              </span>
            </div>
            {rec && !inst?.present && !busy && (
              <div className="font-mono text-[11.5px] text-ink-3">
                {kindLabel(rec.kind)} · {rec.filename}
                {rec.size ? ` · ${bytes(rec.size)}` : ""}
              </div>
            )}

            {inst && (
              <div className="card flex flex-col gap-2.5 p-4 text-[13.5px]">
                {inst.present && inst.external ? (
                  <div className="flex items-center gap-2">
                    <span className="grid h-6 w-6 place-items-center rounded-full bg-ok/15 text-ok">
                      <Icon name="check" className="!h-3.5 !w-3.5" />
                    </span>
                    <span>
                      On this computer{inst.version ? ` · v${inst.version}` : ""}{" "}
                      <span className="text-ink-3">· installed outside the launcher ({inst.external})</span>
                    </span>
                  </div>
                ) : inst.present ? (
                  <>
                    <div className="flex items-center gap-2">
                      <span className="grid h-6 w-6 place-items-center rounded-full bg-ok/15 text-ok">
                        <Icon name="check" className="!h-3.5 !w-3.5" />
                      </span>
                      <span>
                        Installed{inst.version ? ` · v${inst.version}` : ""} <span className="text-ink-3">· {inst.method}</span>
                      </span>
                    </div>
                    <div className="flex flex-wrap items-center gap-2">
                      <label className="flex cursor-pointer items-center gap-2.5 text-ink-2">
                        <Toggle on={inst.desktopShortcut} onChange={(v) => api.setDesktopShortcut(p.id, v).catch(fail)} label="Desktop shortcut" />
                        Desktop shortcut
                      </label>
                      <button className="btn sm danger ml-auto" disabled={busy} onClick={() => uninstall(p)}>
                        <Icon name="trash" /> Uninstall
                      </button>
                    </div>
                  </>
                ) : (
                  <div className="flex items-center gap-2">
                    <Icon name="warn" className="text-mark" />
                    Removed outside the launcher.
                    <button className="btn sm ml-auto" onClick={() => api.forget(p.id)}>
                      Forget it
                    </button>
                  </div>
                )}
              </div>
            )}

            {job?.phase === "failed" && (
              <div className="flex items-start gap-2.5 rounded-xl border border-bad/30 bg-bad/[.06] px-3.5 py-3 text-[13.5px]">
                <Icon name="warn" className="mt-0.5 shrink-0 text-bad" />
                <div className="selectable min-w-0 flex-1">
                  <div>{job.error}</div>
                  {job.hint && <div className="text-ink-3">{job.hint}</div>}
                </div>
                <button className="shrink-0 text-ink-3 hover:text-ink" title="Dismiss" onClick={() => clearJob(p.id)}>
                  <Icon name="x" />
                </button>
              </div>
            )}
            {job?.phase === "cancelled" && <div className="text-[13px] text-ink-3">Stopped. Install again to continue where it left off.</div>}
          </div>
        </div>

        {rest ? (
          <>
          <About p={p} lib={lib} />
  
          <WhatsNew notes={p.releaseNotes} version={p.version} released={p.released} />
  
          <section className="flex flex-col gap-4">
            <div className="flex items-baseline gap-3">
              <h2 className="text-[22px] font-bold">Downloads</h2>
              <span className="text-[12.5px] text-ink-3">checked {ago(p.crawledAt)}</span>
            </div>
            {p.artifacts.length === 0 ? (
              <p className="text-ink-3">
                {p.crawledAt ? "No downloadable files were found. This one is a web tool or source only." : "Looking for downloads…"}
              </p>
            ) : (
              <div className="grid gap-4 md:grid-cols-2">
                {oses.map((o) => (
                  <div key={o} className={`card flex flex-col gap-1 p-2 ${o === lib.host.os || oses.length === 1 ? "md:col-span-2" : ""}`}>
                    <div className="flex items-center gap-2 px-3.5 pt-2.5 pb-1.5 text-[12px] font-semibold tracking-[.12em] text-ink-3 uppercase">
                      <OsIcon os={o} here={o === lib.host.os} size={13} /> {OS_LABEL[o]}
                      {o === lib.host.os && <span className="tracking-normal normal-case text-cyan">· this computer</span>}
                    </div>
                    {byOs.get(o)!.map((a) => (
                      <FileRow key={a.url} a={a} p={p} lib={lib} busy={busy} />
                    ))}
                  </div>
                ))}
              </div>
            )}
          </section>
  
          <BuildPanel p={p} lib={lib} job={job} />
          <div className="pb-10" />
          </>
        ) : (
          <div className="skeleton h-48 rounded-[var(--r)] opacity-30" />
        )}
      </div>
    </div>
  );
}
