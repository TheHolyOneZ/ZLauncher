import { ReactNode } from "react";
import { api, ago, Job, Library, Project } from "../api";
import { Action } from "../components/Action";
import { Icon } from "../components/Icon";
import { Media } from "../components/Media";
import { isActive } from "../state";
import { fail, menu } from "../components/Overlays";
import { projectMenu, uninstall } from "../components/projectMenu";

const METHOD: Record<string, string> = {
  native: "System package",
  converted: "System package (converted)",
  appimage: "AppImage",
  nsis: "Installer",
  msi: "Windows Installer",
  portable: "Portable",
  app: "App",
  external: "Found on this PC",
};

function StatTile({ icon, value, label, tone }: { icon: string; value: number; label: string; tone: "cyan" | "mark" | "ink" }) {
  const color = tone === "cyan" ? "text-cyan bg-cyan/12" : tone === "mark" ? "text-mark bg-mark/15" : "text-ink-2 bg-bg-2";
  return (
    <div className="card flex items-center gap-3.5 p-4">
      <span className={`grid h-10 w-10 place-items-center rounded-xl ${color}`}>
        <Icon name={icon} />
      </span>
      <div>
        <div className="font-head text-[26px] leading-none font-bold">{value}</div>
        <div className="mt-1 text-[12.5px] text-ink-3">{label}</div>
      </div>
    </div>
  );
}

function Row({ p, lib, job, open }: { p: Project; lib: Library; job?: Job; open: (id: string) => void }) {
  const i = p.installed!;
  return (
    <div className="group flex items-center gap-4 px-4 py-3.5 transition-colors hover:bg-hi/40" onContextMenu={(e) => menu(e, projectMenu(p, lib, job, { open: () => open(p.id) }))}>
      <button className="w-[120px] shrink-0 overflow-hidden rounded-lg ring-1 ring-line transition-transform group-hover:scale-[1.02]" onClick={() => open(p.id)}>
        <Media p={p} className="aspect-video" />
      </button>
      <div className="min-w-0 flex-1">
        <button className="truncate text-left text-[16px] font-bold hover:text-cyan" onClick={() => open(p.id)}>
          {p.title}
        </button>
        <div className="mt-1 flex flex-wrap items-center gap-1.5">
          {i.update ? (
            <span className="flex items-center gap-1 font-mono text-[12px]">
              <span className="text-ink-3">{i.version ? `v${i.version}` : "?"}</span>
              <Icon name="back" className="!h-3 !w-3 rotate-180 text-mark" />
              <span className="font-medium text-mark">v{i.update}</span>
            </span>
          ) : (
            i.version && <span className="font-mono text-[12px] text-ink-2">v{i.version}</span>
          )}
          <span className="tag">{METHOD[i.method] ?? i.method}</span>
          {i.external && <span className="tag">via {i.external}</span>}
          {!i.external && i.installedAt > 0 && <span className="text-[12px] text-ink-3">· {ago(i.installedAt)}</span>}
        </div>
        {!i.present && <div className="mt-1 text-[12.5px] text-mark">Removed outside the launcher.</div>}
      </div>
      <div className="flex shrink-0 items-center gap-2">
        {i.present ? (
          <>
            <Action p={p} lib={lib} job={job} compact />
            {i.update && !isActive(job) && (
              <button className="btn sm" onClick={() => api.launch(p.id).catch(fail)} title="Open">
                <Icon name="play" />
              </button>
            )}
            {!isActive(job) && !i.external && (
              <>
                <button
                  className={`btn sm icon ${i.desktopShortcut ? "text-cyan" : ""}`}
                  title={i.desktopShortcut ? "Remove the desktop shortcut" : "Add a desktop shortcut"}
                  onClick={() => api.setDesktopShortcut(p.id, !i.desktopShortcut).catch(fail)}
                >
                  <Icon name="monitor" />
                </button>
                <button className="btn sm icon danger" title="Uninstall" onClick={() => uninstall(p)}>
                  <Icon name="trash" />
                </button>
              </>
            )}
          </>
        ) : (
          <button className="btn sm" onClick={() => api.forget(p.id)}>
            Forget
          </button>
        )}
      </div>
    </div>
  );
}

function Group({ title, note, children }: { title: string; note?: string; children: ReactNode }) {
  return (
    <section className="flex flex-col gap-2">
      <div className="flex items-baseline gap-2 px-1">
        <h2 className="text-[12px] font-semibold tracking-[.14em] text-ink-3 uppercase">{title}</h2>
        {note && <span className="text-[12.5px] text-ink-3">· {note}</span>}
      </div>
      <div className="card divide-y divide-line overflow-hidden">{children}</div>
    </section>
  );
}

export function Installed({ lib, jobs, open }: { lib: Library; jobs: Record<string, Job>; open: (id: string) => void }) {
  const mine = lib.projects.filter((p) => p.installed);
  const updates = mine.filter((p) => p.installed!.present && p.installed!.update);
  const ours = mine.filter((p) => !p.installed!.external && !p.installed!.update);
  const found = mine.filter((p) => p.installed!.external && !p.installed!.update);
  const runnable = updates.filter((p) => !isActive(jobs[p.id]));

  if (mine.length === 0) {
    return (
      <div className="page flex flex-col items-center gap-4 py-32 text-center">
        <span className="grid h-16 w-16 place-items-center rounded-2xl bg-hi text-cyan">
          <Icon name="box" className="!h-7 !w-7" />
        </span>
        <div>
          <div className="text-[18px] font-bold">Your library is empty</div>
          <p className="text-[14px] text-ink-3">Pick a tool in Discover and press Install. Tools you already have show up here on their own.</p>
        </div>
      </div>
    );
  }

  const row = (p: Project) => <Row key={p.id} p={p} lib={lib} job={jobs[p.id]} open={open} />;
  return (
    <div className="page mx-auto flex max-w-[1080px] flex-col gap-7 px-8 py-8">
      <div className="flex flex-wrap items-end gap-4">
        <div className="flex-1">
          <h1 className="text-[32px] font-bold">Library</h1>
          <p className="text-[14px] text-ink-3">Everything from zsync on this computer.</p>
        </div>
        {runnable.length > 0 && (
          <button className="btn upd lg" onClick={() => runnable.forEach((p) => api.install(p.id).catch(console.error))}>
            <Icon name="up" /> Update all ({runnable.length})
          </button>
        )}
      </div>

      <div className="grid grid-cols-3 gap-3">
        <StatTile icon="box" value={mine.length} label="tools on this computer" tone="cyan" />
        <StatTile icon="up" value={updates.length} label={updates.length === 1 ? "update ready" : "updates ready"} tone={updates.length ? "mark" : "ink"} />
        <StatTile icon="search" value={mine.filter((p) => p.installed!.external).length} label="found already installed" tone="ink" />
      </div>

      {updates.length > 0 && <Group title="Updates available">{updates.map(row)}</Group>}
      {ours.length > 0 && <Group title="Installed with ZLauncher">{ours.map(row)}</Group>}
      {found.length > 0 && (
        <Group title="Found on this computer" note="installed before or by hand; updates still work">
          {found.map(row)}
        </Group>
      )}
    </div>
  );
}
