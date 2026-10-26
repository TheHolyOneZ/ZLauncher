import { useEffect, useState } from "react";
import { api, bytes, Library, OS_LABEL, SelfUpdate, Settings } from "../api";
import { Toggle } from "../components/Controls";
import { Icon } from "../components/Icon";
import logo from "../assets/logo.png";

export const HOMEPAGE = "https://zsync.eu/zlauncher/";
export const SITE = "https://zsync.eu/";
export const AUTHOR = "https://github.com/TheHolyOneZ";
export const SOURCE = "https://github.com/TheHolyOneZ/ZLauncher";

const inTauri = "__TAURI_INTERNALS__" in window;

function useAppVersion(): string {
  const [v, setV] = useState("0.2.0");
  useEffect(() => {
    if (inTauri) import("@tauri-apps/api/app").then((m) => m.getVersion()).then(setV, () => {});
  }, []);
  return v;
}

function LinkCard({ icon, title, note, url }: { icon: string; title: string; note: string; url: string }) {
  return (
    <button type="button" className="card group flex items-start gap-3.5 p-4 text-left transition-colors hover:border-line-2 hover:bg-hi" onClick={() => api.openUrl(url)}>
      <span className="grid h-9 w-9 shrink-0 place-items-center rounded-[11px] bg-gradient-to-br from-cyan/20 to-violet/20 text-cyan ring-1 ring-cyan/20">
        <Icon name={icon} className="!h-[16px] !w-[16px]" />
      </span>
      <span className="min-w-0 flex-1">
        <span className="flex items-center gap-1.5 text-[14.5px] font-semibold">
          {title}
          <Icon name="external" className="!h-3 !w-3 text-ink-3 opacity-0 transition-opacity group-hover:opacity-100" />
        </span>
        <span className="mt-0.5 block text-[13px] leading-snug text-ink-3">{note}</span>
        <span className="mt-1.5 block truncate font-mono text-[11.5px] text-ink-3">{url.replace(/^https:\/\//, "").replace(/\/$/, "")}</span>
      </span>
    </button>
  );
}

function UpdateCard({ u, auto, setAuto }: { u: SelfUpdate | null; auto: boolean; setAuto: (v: boolean) => void }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const st = u?.state ?? "idle";
  const found = !!u?.version && ["available", "downloading", "ready", "installing"].includes(st);
  const run = (f: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    f()
      .catch((e) => setError(String(e)))
      .finally(() => setBusy(false));
  };
  const pct = u?.total ? Math.round((u.done / u.total) * 100) : 0;

  return (
    <div className={`card divide-y divide-line overflow-hidden ${found ? "border-cyan/30" : ""}`}>
      <div className={`flex items-center gap-4 p-5 ${found ? "bg-[color-mix(in_srgb,var(--cyan)_6%,var(--card))]" : ""}`}>
        <span className={`grid h-9 w-9 shrink-0 place-items-center rounded-xl ${found ? "bg-cyan/15 text-cyan" : "bg-ok/12 text-ok"}`}>
          <Icon name={found ? "up" : st === "error" ? "warn" : "check"} className={st === "checking" ? "spin" : ""} />
        </span>
        <div className="min-w-0 flex-1">
          <div className="text-[15px] font-semibold">
            {found
              ? st === "ready"
                ? `ZLauncher ${u!.version} is ready to install`
                : st === "installing"
                  ? `Installing ZLauncher ${u!.version}…`
                  : `ZLauncher ${u!.version} is out`
              : st === "checking"
                ? "Looking for updates…"
                : st === "error"
                  ? u?.version
                    ? `ZLauncher ${u.version} couldn't be downloaded`
                    : "Couldn't look for updates"
                  : "You have the newest version"}
          </div>
          <div className="mt-0.5 text-[13px] text-ink-3">
            {st === "downloading"
              ? `Downloading ${bytes(u!.done)}${u!.total ? ` of ${bytes(u!.total)}` : ""}`
              : found && !u!.canInstall
                ? `${u!.reason ?? ""} Get the new version from the homepage.`
                : found
                  ? `You have ${u!.current}.${st === "ready" ? " The launcher restarts into the new version." : ""}`
                  : st === "error"
                    ? (u?.error ?? "")
                    : `Version ${u?.current ?? ""}.`}
          </div>
          {st === "downloading" && (
            <div className="bar mt-2.5">
              <i style={{ width: `${pct}%` }} />
            </div>
          )}
          {(error || (found && u?.notes)) && <div className={`mt-2 text-[13px] ${error ? "text-bad" : "text-ink-2"}`}>{error ?? u!.notes}</div>}
        </div>
        {found && !u!.canInstall ? (
          <button className="btn upd sm shrink-0" onClick={() => api.openUrl(HOMEPAGE)}>
            <Icon name="download" /> Homepage
          </button>
        ) : found ? (
          <button className="btn upd sm shrink-0" disabled={busy || st === "downloading" || st === "installing"} onClick={() => run(api.selfUpdateInstall)}>
            <Icon name="refresh" className={st === "installing" ? "spin" : ""} /> {st === "ready" ? "Restart to update" : "Update now"}
          </button>
        ) : (
          <button className="btn sm shrink-0" disabled={busy || st === "checking"} onClick={() => run(api.selfUpdateCheck)}>
            <Icon name="refresh" className={st === "checking" ? "spin" : ""} /> Check
          </button>
        )}
      </div>
      <label className="flex cursor-pointer items-center gap-5 px-5 py-4">
        <div className="min-w-0 flex-1">
          <div className="text-[14.5px] font-semibold">Update ZLauncher automatically</div>
          <div className="mt-0.5 text-[13px] leading-snug text-ink-3">New versions are downloaded in the background and installed when you restart from the title bar. Every update is signed and checked first.</div>
        </div>
        <Toggle on={auto} onChange={setAuto} label="Update ZLauncher automatically" />
      </label>
    </div>
  );
}

export function AboutPage({
  lib,
  update,
  settings,
  save,
}: {
  lib: Library;
  open: (id: string) => void;
  update: SelfUpdate | null;
  settings: Settings;
  save: (s: Settings) => void;
}) {
  const appVersion = useAppVersion();
  const version = update?.current || appVersion;
  const [copied, setCopied] = useState(false);
  const h = lib.host;

  const info = [
    `ZLauncher ${version}`,
    `${h.name || OS_LABEL[h.os]} (${h.os}, ${h.arch}${h.linuxFamily ? `, ${h.linuxFamily} family` : ""})`,
    `Videos: ${h.video ? "on" : "off"}`,
    `Tools listed: ${lib.projects.length}`,
  ].join("\n");
  const copy = () => {
    navigator.clipboard.writeText(info).then(() => {
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1800);
    });
  };

  return (
    <div className="page mx-auto flex max-w-[860px] flex-col gap-8 px-8 py-8">
      <div className="card relative overflow-hidden p-8">
        <div className="glow -top-28 -left-20 h-72 w-72 [--glow:color-mix(in_srgb,var(--cyan)_18%,transparent)]" />
        <div className="glow -right-24 -bottom-32 h-72 w-72 [--glow:color-mix(in_srgb,var(--violet)_16%,transparent)]" />
        <div className="relative flex items-center gap-6">
          <img src={logo} alt="" className="h-[88px] w-[88px] rounded-[22px] shadow-[0_14px_36px_-10px_rgba(63,198,251,.55)]" draggable={false} />
          <div className="min-w-0 flex-1">
            <div className="flex flex-wrap items-center gap-2.5">
              <h1 className="text-[34px] leading-none font-bold">ZLauncher</h1>
              <span className="rounded-full border border-line-2 px-2.5 py-0.5 font-mono text-[12px] text-ink-2">v{version}</span>
            </div>
            <p className="mt-2.5 max-w-[56ch] text-[14.5px] leading-relaxed text-ink-2">
              Every ZSync tool in one place. It finds the right download for your system and installs, updates, launches and removes it with one click.
            </p>
            <div className="mt-4 flex flex-wrap gap-2">
              <button className="btn go" onClick={() => api.openUrl(HOMEPAGE)}>
                <Icon name="globe" /> Homepage
              </button>
              <button className="btn" onClick={() => api.openUrl(SOURCE)}>
                <Icon name="code" /> Source code
              </button>
            </div>
          </div>
        </div>
      </div>

      <section className="flex flex-col gap-3">
        <h2 className="px-1 text-[17px] font-bold">Updates</h2>
        <UpdateCard u={update} auto={settings.autoUpdate} setAuto={(v) => save({ ...settings, autoUpdate: v })} />
      </section>

      <section className="flex flex-col gap-3">
        <h2 className="px-1 text-[17px] font-bold">Links</h2>
        <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
          <LinkCard icon="download" title="ZLauncher homepage" note="The launcher's own page. Download or reinstall it for Windows, Linux and macOS." url={HOMEPAGE} />
          <LinkCard icon="globe" title="zsync.eu" note="The home of all ZSync tools. The launcher shows the same list, live from the site." url={SITE} />
          <LinkCard icon="user" title="TheHolyOneZ" note="The author, on GitHub: all the tools and their source code." url={AUTHOR} />
          <LinkCard icon="bug" title="Report a problem" note="Something not working? Open an issue, with the system details below." url={`${SOURCE}/issues`} />
        </div>
      </section>

      <section className="flex flex-col gap-3">
        <h2 className="px-1 text-[17px] font-bold">License</h2>
        <div className="card flex items-start gap-4 p-5">
          <span className="grid h-9 w-9 shrink-0 place-items-center rounded-[11px] bg-gradient-to-br from-cyan/20 to-violet/20 text-cyan ring-1 ring-cyan/20">
            <Icon name="scale" className="!h-[16px] !w-[16px]" />
          </span>
          <div className="min-w-0 flex-1 text-[13.5px] leading-relaxed text-ink-2">
            <div className="text-[15px] font-semibold text-ink">GNU General Public License v3.0 or later</div>
            <p className="mt-1">
              ZLauncher is free and open-source software, © 2026 TheHolyOneZ. You may use, study, share and change it. If you pass on a changed version, it has to stay
              under the same license, with its source code. It comes without any warranty.
            </p>
            <p className="mt-2 text-ink-3">Each tool it installs has its own license, shown on that tool's page.</p>
            <div className="mt-3 flex flex-wrap gap-2">
              <button className="btn sm" onClick={() => api.openUrl("https://www.gnu.org/licenses/gpl-3.0.html")}>
                <Icon name="external" /> Read the license
              </button>
            </div>
          </div>
        </div>
      </section>

      <section className="flex flex-col gap-3">
        <h2 className="px-1 text-[17px] font-bold">This system</h2>
        <div className="card flex items-start gap-4 p-5">
          <pre className="selectable min-w-0 flex-1 overflow-x-auto font-mono text-[12.5px] leading-relaxed text-ink-2">{info}</pre>
          <button className="btn sm shrink-0" onClick={copy}>
            <Icon name={copied ? "check" : "copy"} /> {copied ? "Copied" : "Copy"}
          </button>
        </div>
        <p className="px-1 text-[12.5px] text-ink-3">Built with Tauri, Rust and React. Install engine for Linux based on ZLynstall.</p>
      </section>
    </div>
  );
}
