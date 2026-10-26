import { Profiler, useCallback, useEffect, useMemo, useRef, useState, useTransition } from "react";
import { invoke } from "@tauri-apps/api/core";
import { AdminStatus, api, ago, canPlayVideo, Job, SelfUpdate, Settings } from "./api";
import { Icon } from "./components/Icon";
import { AdminDialog } from "./components/AdminDialog";
import { ResizeEdges, TitleBar, useSystemFrame } from "./components/TitleBar";
import { Toasts } from "./components/Toasts";
import { Overlays } from "./components/Overlays";
import { devNote, markAction, onRender } from "./perf";
import { Browse, Filters, Sort } from "./pages/Browse";
import { Downloads } from "./pages/Downloads";
import { Installed } from "./pages/Installed";
import { ProjectPage } from "./pages/ProjectPage";
import { AboutPage } from "./pages/AboutPage";
import { SettingsPage } from "./pages/SettingsPage";
import { Assistant } from "./components/Assistant";
import { isActive, useJobs, useLibrary } from "./state";
import logo from "./assets/logo.png";
import assistantGlyph from "./assets/assistant-glyph.png";

type Tab = "browse" | "installed" | "downloads" | "settings" | "about";

function parked(away: boolean): { className: string; "aria-hidden"?: boolean } {
  return away ? { className: "discover-layer discover-away", "aria-hidden": true } : { className: "discover-layer" };
}

const NO_FILTERS: Filters = { q: "", os: "all", cat: null, retired: false, fav: false };

export default function App() {
  const { library, error: libraryError } = useLibrary();
  const { jobs, history, clear } = useJobs();
  const [openId, setOpenIdNow] = useState<string | null>(null);
  const [tab, setTabNow] = useState<Tab>("browse");
  const [navigating, startNav] = useTransition();
  const setOpenId = useCallback((id: string | null) => {
    markAction(id ? `open ${id}` : "back");
    startNav(() => setOpenIdNow(id));
  }, []);
  const setTab = useCallback((t: Tab) => {
    markAction(`tab ${t}`);
    startNav(() => setTabNow(t));
  }, []);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [filters, setFiltersNow] = useState<Filters>(NO_FILTERS);
  const setFilters = useCallback((f: Filters) => {
    markAction(`filter os=${f.os} cat=${f.cat ?? "-"} q=${f.q ? "…" : "-"} fav=${f.fav}`);
    setFiltersNow(f);
  }, []);
  const search = useRef<HTMLInputElement>(null);
  const main = useRef<HTMLElement>(null);

  useEffect(() => {
    api.settings().then(setSettings).catch((e) => setError(String(e)));
    const h = new URLSearchParams(location.hash.slice(1));
    if (h.get("p")) setOpenId(h.get("p"));
    if (h.get("tab")) setTab(h.get("tab") as Tab);
    if (h.has("still")) document.documentElement.classList.add("still");
    if (h.has("fav")) setFilters({ ...NO_FILTERS, fav: true });
    if (h.has("assistant")) setAssistantOpen(true);
  }, []);

  useEffect(() => {
    const t = settings?.theme;
    if (t === "dark" || t === "light") document.documentElement.dataset.theme = t;
    else delete document.documentElement.dataset.theme;
  }, [settings?.theme]);

  useSystemFrame(!!settings?.systemFrame, library?.host.os ?? "linux");

  const [admin, setAdmin] = useState<AdminStatus | null>(null);
  const [adminOpen, setAdminOpen] = useState(false);
  const [assistantOpen, setAssistantOpen] = useState(false);
  const adminWaiter = useRef<((unlocked: boolean) => void) | null>(null);
  const refreshAdmin = useCallback(() => {
    api.adminStatus().then(setAdmin).catch(() => {});
  }, []);
  useEffect(() => {
    refreshAdmin();
    const t = window.setInterval(refreshAdmin, 60_000);
    return () => window.clearInterval(t);
  }, [refreshAdmin]);
  const offered = useRef(false);
  useEffect(() => {
    if (offered.current || !admin || !settings) return;
    offered.current = true;
    if (admin.supported && !admin.unlocked && settings.askAdminOnStart) {
      const t = window.setTimeout(() => setAdminOpen(true), 900);
      return () => window.clearTimeout(t);
    }
  }, [admin, settings]);
  const toggleAdmin = useCallback(() => {
    if (admin?.unlocked) api.adminLock().then(refreshAdmin);
    else setAdminOpen(true);
  }, [admin, refreshAdmin]);
  const requestAssistantAdmin = useCallback(() => new Promise<boolean>((resolve) => {
    adminWaiter.current = resolve;
    setAdminOpen(true);
  }), []);

  const [selfUpdate, setSelfUpdate] = useState<SelfUpdate | null>(null);
  useEffect(() => {
    api.selfUpdateStatus().then(setSelfUpdate).catch(() => {});
    const off = api.onSelfUpdate(setSelfUpdate);
    return () => {
      off.then((f) => f());
    };
  }, []);
  const updateWaiting = !!selfUpdate?.version && ["available", "downloading", "ready", "installing"].includes(selfUpdate.state);

  const browseView = tab === "browse" && !openId;
  const browseScroll = useRef(0);
  useEffect(() => {
    main.current?.scrollTo({ top: browseView ? browseScroll.current : 0 });
  }, [browseView, tab, openId]);
  useEffect(() => {
    browseScroll.current = 0;
    if (browseView) main.current?.scrollTo({ top: 0 });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [filters.cat, filters.fav, filters.os]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const typing = e.target instanceof HTMLInputElement || e.target instanceof HTMLSelectElement;
      if ((e.key === "k" && (e.ctrlKey || e.metaKey)) || (e.key === "/" && !typing)) {
        e.preventDefault();
        setTab("browse");
        setOpenId(null);
        search.current?.focus();
        search.current?.select();
      } else if (e.key === "Escape") {
        if (assistantOpen) setAssistantOpen(false);
        else if (openId) setOpenId(null);
        else if (typing) (e.target as HTMLElement).blur();
        else if (filters.q || filters.cat || filters.fav || filters.os !== "all") setFilters(NO_FILTERS);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [openId, filters, assistantOpen]);

  const counts = useMemo(() => {
    const m = new Map<string, number>();
    for (const p of library?.projects ?? []) if (p.category && !p.retired) m.set(p.category, (m.get(p.category) ?? 0) + 1);
    return [...m.entries()].sort((a, b) => b[1] - a[1]);
  }, [library?.projects]);

  useEffect(() => {
    if (!import.meta.env.DEV || !library || !new URLSearchParams(location.hash.slice(1)).has("autopilot")) return;
    const ids = library.projects.filter((p) => p.video).slice(0, 4).map((p) => p.id);
    let n = 0;
    const first = Number(new URLSearchParams(location.hash.slice(1)).get("autopilot")) || 3000;
    const every = Number(new URLSearchParams(location.hash.slice(1)).get("every")) || 3000;
    let t = 0;
    const start = window.setTimeout(() => {
      t = window.setInterval(() => {
      const step = n++;
      const q = new URLSearchParams(location.hash.slice(1));
      if (q.has("cards")) {
        const cards = [...document.querySelectorAll<HTMLElement>(".discover-layer .tool")].slice(0, 10);
        const from = cards[(step - 1 + cards.length) % cards.length];
        const to = cards[step % cards.length];
        if (step > 0) from?.dispatchEvent(new MouseEvent("mouseout", { bubbles: true, relatedTarget: to }));
        to?.dispatchEvent(new MouseEvent("mouseover", { bubbles: true, relatedTarget: from }));
        markAction(`hover card ${step % cards.length}`);
        devNote(`hover card ${step % cards.length}, ${document.querySelectorAll("video").length} video elements`);
        return;
      }
      if (q.has("tabs")) setTab(step % 2 === 0 ? "settings" : "browse");
      else if (step % 2 === 0) setOpenId(ids[(step / 2) % ids.length]);
      else setOpenId(null);
      }, every);
    }, first - every);
    return () => {
      window.clearTimeout(start);
      window.clearInterval(t);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [!!library]);

  const ready = !!library && !!settings;
  useEffect(() => {
    if (ready) {
      requestAnimationFrame(() =>
        invoke("log", { message: `ready: ${library!.projects.length} tools, video ${canPlayVideo ? "on" : "off"}${location.hash ? `, view ${location.hash}` : ""}` }).catch(() => {}),
      );
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ready]);

  const save = (s: Settings) => {
    setSettings(s);
    api.setSettings(s).catch(console.error);
  };
  const latest = useRef(settings);
  latest.current = settings;
  const toggleFav = useCallback((id: string) => {
    const s = latest.current;
    if (!s) return;
    const favs = s.favorites ?? [];
    const next = { ...s, favorites: favs.includes(id) ? favs.filter((f) => f !== id) : [...favs, id] };
    setSettings(next);
    api.setSettings(next).catch(console.error);
  }, []);
  const setSort = useCallback((sort: Sort) => {
    const s = latest.current;
    if (!s) return;
    const next = { ...s, sort };
    setSettings(next);
    api.setSettings(next).catch(console.error);
  }, []);

  if (!library || !settings) {
    return (
      <div className="grid h-full place-items-center">
        <div className="flex flex-col items-center gap-4 text-ink-3">
          <img src={logo} alt="" className="h-12 w-12 animate-pulse" />
          <div className="selectable max-w-[60ch] text-center text-[13px]">{error ?? libraryError ?? ""}</div>
        </div>
      </div>
    );
  }

  const updates = library.projects.filter((p) => p.installed?.present && p.installed.update).length;
  const installed = library.projects.filter((p) => p.installed).length;
  const active = Object.values(jobs).filter(isActive).length;
  const project = openId ? library.projects.find((p) => p.id === openId) : undefined;
  const go = (t: Tab) => {
    setTab(t);
    setOpenId(null);
  };
  const pickCategory = (c: string | null) => {
    setFilters({ ...filters, cat: c, fav: false });
    go("browse");
  };
  const favorites = settings.favorites ?? [];
  const browsing = tab === "browse" && !project;

  const nav: [Tab, string, string, number, string][] = [
    ["browse", "Discover", "grid", 0, ""],
    ["installed", "Library", "box", updates || installed, updates ? "bg-mark text-[#1b1505]" : "bg-line-2 text-ink-2"],
    ["downloads", "Downloads", "download", active, "bg-cyan text-[#04131c]"],
    ["settings", "Settings", "gear", 0, ""],
    ["about", "About", "info", updateWaiting ? 1 : 0, "bg-cyan text-[#04131c]"],
  ];

  const where = project
    ? `${filters.fav ? "Favorites" : (filters.cat ?? "Discover")}  ›  ${project.title}`
    : tab === "browse"
      ? filters.fav
        ? "Favorites"
        : (filters.cat ?? (filters.q ? `Search: ${filters.q}` : "Discover"))
      : { installed: "Library", downloads: "Downloads", settings: "Settings", about: "About", browse: "Discover" }[tab];
  const ownFrame = !settings.systemFrame || library.host.os === "macos";

  return (
    <div className={`flex h-full flex-col ${ownFrame && library.host.os !== "macos" ? "frameless" : ""}`}>
      {ownFrame && <TitleBar os={library.host.os} where={where} admin={admin} onAdmin={toggleAdmin} update={updateWaiting ? selfUpdate : null} onUpdate={() => (selfUpdate?.state === "ready" && selfUpdate.canInstall ? api.selfUpdateInstall().catch(() => go("about")) : go("about"))} />}
      {ownFrame && library.host.os !== "macos" && <ResizeEdges />}
      <div className="flex min-h-0 flex-1">
      <aside className="flex w-[248px] shrink-0 flex-col gap-5 border-r border-line bg-side px-3.5 pt-4 pb-4">

        <label className="relative flex items-center">
          <Icon name="search" className="pointer-events-none absolute left-3 text-ink-3" />
          <input
            ref={search}
            className="input w-full pr-14 pl-9 text-[13.5px]"
            placeholder="Search tools"
            value={filters.q}
            onChange={(e) => {
              setFilters({ ...filters, q: e.target.value });
              setTab("browse");
              setOpenId(null);
            }}
          />
          <span className="pointer-events-none absolute right-2.5 flex gap-0.5">
            <span className="kbd">Ctrl</span>
            <span className="kbd">K</span>
          </span>
        </label>

        <nav className="flex flex-col gap-0.5">
          {nav.map(([t, label, icon, badge, badgeClass]) => (
            <div key={t} className="contents">
              <button key={t} className="nav-item" aria-current={tab === t && !openId && (t !== "browse" || (!filters.cat && !filters.fav))} onClick={() => (t === "browse" ? pickCategory(null) : go(t))}>
                <Icon name={icon} />
                {label}
                {badge > 0 && <span className={`ml-auto rounded-full px-1.5 py-px font-mono text-[10.5px] ${badgeClass}`}>{badge}</span>}
              </button>
              {t === "browse" && (
                <button
                  className="nav-item"
                  aria-current={tab === "browse" && !openId && filters.fav}
                  onClick={() => {
                    setFilters({ ...NO_FILTERS, fav: true });
                    go("browse");
                  }}
                >
                  <Icon name="heart" />
                  Favorites
                  {favorites.length > 0 && <span className="ml-auto font-mono text-[11px] text-ink-3">{favorites.length}</span>}
                </button>
              )}
              {t === "browse" && settings.assistantEnabled && (
                <button className="nav-item" onClick={() => setAssistantOpen(true)}>
                  <img src={assistantGlyph} alt="" draggable={false} className="-mx-px h-[18px] w-[18px] shrink-0" /> Assistant
                  <span className="ml-auto text-[10.5px] text-ink-3">NEW</span>
                </button>
              )}
            </div>
          ))}
        </nav>

        <div className="scroll -mx-1 flex min-h-0 flex-1 flex-col gap-0.5 px-1">
          <div className="px-2.5 pb-1.5 text-[11px] font-semibold tracking-[.14em] text-ink-3 uppercase">Categories</div>
          {counts.map(([c, n]) => (
            <button key={c} className="nav-item !h-8 text-[13.5px]" aria-current={tab === "browse" && !openId && filters.cat === c} onClick={() => pickCategory(filters.cat === c ? null : c)}>
              <span className="truncate">{c}</span>
              <span className="ml-auto font-mono text-[11px] text-ink-3">{n}</span>
            </button>
          ))}
        </div>

        <div className="flex flex-col gap-2 rounded-xl border border-line px-3 py-2.5 text-[12px] text-ink-3">
          {library.crawling ? (
            <>
              <div className="flex items-center justify-between">
                <span>Checking websites</span>
                <span className="font-mono">
                  {library.crawling[0]}/{library.crawling[1]}
                </span>
              </div>
              <div className="bar">
                <i style={{ width: `${(library.crawling[0] / Math.max(library.crawling[1], 1)) * 100}%` }} />
              </div>
            </>
          ) : library.catalogueError ? (
            <div className="flex items-center gap-1.5 text-mark" title={library.catalogueError}>
              <Icon name="warn" /> Offline · saved list
            </div>
          ) : (
            <div className="flex items-center justify-between">
              <span className="flex items-center gap-1.5">
                <span className="h-1.5 w-1.5 rounded-full bg-ok" /> Up to date
              </span>
              <span>{ago(library.fetchedAt)}</span>
            </div>
          )}
          <button className="flex items-center gap-1.5 self-start hover:text-ink" onClick={() => api.refresh(true)}>
            <Icon name="refresh" className={`!h-3.5 !w-3.5 ${library.crawling ? "spin" : ""}`} /> Check now
          </button>
        </div>
      </aside>

      <main
        ref={main}
        className="scroll relative flex-1 [scrollbar-gutter:stable]"
        onScroll={(e) => {
          if (browseView) browseScroll.current = e.currentTarget.scrollTop;
        }}
      >
        {library.projects.length === 0 ? (
          <div className="grid h-full place-items-center text-center text-ink-3">
            {library.catalogueError ? (
              <div className="flex flex-col items-center gap-2">
                <Icon name="warn" className="!h-8 !w-8 text-mark" />
                <p>Couldn't load the tool list from zsync.eu.</p>
                <p className="font-mono text-[12px]">{library.catalogueError}</p>
                <button className="btn mt-3" onClick={() => api.refresh(true)}>
                  Try again
                </button>
              </div>
            ) : (
              <div className="flex flex-col items-center gap-3">
                <img src={logo} alt="" className="h-10 w-10 animate-pulse" />
                Loading the tool list from zsync.eu…
              </div>
            )}
          </div>
        ) : (
          <>
            <div {...parked(!browsing)}>
              <Profiler id="Browse" onRender={onRender}>
              <Browse
                active={browsing}
                lib={library}
                jobs={jobs}
                hover={settings.hoverVideo}
                filters={filters}
                setFilters={setFilters}
                open={setOpenId}
                favorites={favorites}
                toggleFav={toggleFav}
                sort={(settings.sort as Sort) ?? "newest"}
                setSort={setSort}
              />
              </Profiler>
            </div>
            {project ? (
          <Profiler id="ProjectPage" onRender={onRender}>
          <ProjectPage
            key={project.id}
            p={project}
            lib={library}
            job={jobs[project.id]}
            back={() => setOpenId(null)}
            clearJob={clear}
            fav={favorites.includes(project.id)}
            toggleFav={() => toggleFav(project.id)}
          />
          </Profiler>
        ) : tab === "browse" ? null : tab === "installed" ? (
          <Installed lib={library} jobs={jobs} open={setOpenId} />
        ) : tab === "downloads" ? (
          <Downloads jobs={jobs} history={history} lib={library} clear={clear} open={setOpenId} />
        ) : tab === "about" ? (
          <AboutPage lib={library} open={setOpenId} update={selfUpdate} settings={settings} save={save} />
        ) : (
          <SettingsPage lib={library} settings={settings} save={save} admin={admin} onAdmin={toggleAdmin} />
        )}
          </>
        )}
      </main>

      </div>
      {navigating && <div className="navbar" aria-hidden />}
      {adminOpen && (
        <AdminDialog
          askOnStart={settings.askAdminOnStart}
          setAskOnStart={(v) => save({ ...settings, askAdminOnStart: v })}
          done={(unlocked) => {
            setAdminOpen(false);
            refreshAdmin();
            adminWaiter.current?.(unlocked);
            adminWaiter.current = null;
          }}
        />
      )}
      {assistantOpen && <Assistant lib={library} jobs={jobs} close={() => setAssistantOpen(false)} openProject={(id) => { setAssistantOpen(false); go("browse"); setOpenId(id); }} openSettings={() => { setAssistantOpen(false); go("settings"); }} admin={admin} requestAdmin={requestAssistantAdmin} settings={settings} save={save} titleBar={ownFrame} />}
      <Toasts jobs={jobs} open={(id: string) => setOpenId(id)} onDismiss={(j: Job) => clear(j.projectId)} />
      <Overlays />
    </div>
  );
}
