import { lazy, Profiler, ReactNode, Suspense, useCallback, useEffect, useMemo, useRef, useState, useTransition } from "react";
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
import { TalkToZ } from "./components/TalkToZ";
import { isActive, useJobs, useLibrary } from "./state";
import { EffectsProvider, useEffectsLevel } from "./effects";
import { NavIndicator } from "./components/NavIndicator";
import { Block, blockHidden, blockOrder } from "./sidebar";
import ClickSpark from "./components/bits/ClickSpark/ClickSpark";
import logo from "./assets/logo.png";
import assistantGlyph from "./assets/assistant-glyph.png";
import ttzIcon from "./assets/ttz.png";

const Backdrop = lazy(() => import("./components/Backdrop"));

function FxLayer() {
  const { level } = useEffectsLevel();
  const [warm, setWarm] = useState(false);
  useEffect(() => {
    let idle = 0;
    const t = window.setTimeout(() => {
      if ("requestIdleCallback" in window) idle = window.requestIdleCallback(() => setWarm(true), { timeout: 1500 });
      else setWarm(true);
    }, 900);
    return () => {
      window.clearTimeout(t);
      if (idle && "cancelIdleCallback" in window) window.cancelIdleCallback(idle);
    };
  }, []);
  if (level === "off") return null;
  return (
    <div className="fx-backdrop left-[248px]" aria-hidden>
      {warm && (
        <Suspense fallback={null}>
          <Backdrop />
        </Suspense>
      )}
    </div>
  );
}

function dismissBoot(): () => void {
  const boot = document.getElementById("boot");
  if (!boot) return () => {};
  let frame = requestAnimationFrame(() => {
    frame = requestAnimationFrame(() => boot.classList.add("out"));
  });
  const gone = window.setTimeout(() => boot.remove(), 900);
  return () => {
    cancelAnimationFrame(frame);
    window.clearTimeout(gone);
  };
}

function Sparks({ children }: { children: ReactNode }) {
  const { motion, palette } = useEffectsLevel();
  return (
    <ClickSpark sparkColor={[palette.cyan, palette.violet]} sparkSize={7} sparkRadius={18} sparkCount={9} duration={420} shape="streak" glow={0.6} disabled={!motion} className="!h-full">
      {children}
    </ClickSpark>
  );
}

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
    if (h.has("ttz")) setTtzOpen(true);
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
  const [ttzOpen, setTtzOpen] = useState(false);
  const [ttzUnread, setTtzUnread] = useState(0);
  const [ttzStart, setTtzStart] = useState<string | null>(null);
  const [ttzReply, setTtzReply] = useState<{ id: string; text: string } | null>(null);
  const checkTtz = useCallback(() => {
    api.ttzState(false).then((s) => setTtzUnread(s.chats.filter((c) => c.unread).length)).catch(() => {});
  }, []);
  useEffect(() => {
    checkTtz();
    const off = api.onTtz(checkTtz);
    const replies = api.onTtzReply((r) => {
      setTtzReply(r);
      window.setTimeout(() => setTtzReply((cur) => (cur === r ? null : cur)), 9000);
    });
    return () => {
      off.then((f) => f());
      replies.then((f) => f());
    };
  }, [checkTtz]);
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
        if (ttzOpen) setTtzOpen(false);
        else if (assistantOpen) setAssistantOpen(false);
        else if (openId) setOpenId(null);
        else if (typing) (e.target as HTMLElement).blur();
        else if (filters.q || filters.cat || filters.fav || filters.os !== "all") setFilters(NO_FILTERS);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [openId, filters, assistantOpen, ttzOpen]);

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
  }, [!!library]);

  useEffect(() => {
    if (!import.meta.env.DEV || !library || !location.hash.includes("sweep")) return;
    const start = performance.now() + 2500;
    let frame = 0;
    const step = (now: number) => {
      frame = requestAnimationFrame(step);
      if (now < start) return;
      const t = (now - start) / 1000;
      const el = main.current;
      if (el && location.hash.includes("sweep=scroll")) el.scrollTop = (Math.sin(t * 0.9) * 0.5 + 0.5) * Math.min(2400, el.scrollHeight - el.clientHeight);
      const x = window.innerWidth * (0.55 + 0.35 * Math.sin(t * 1.3));
      const y = window.innerHeight * (0.5 + 0.35 * Math.cos(t * 1.7));
      window.dispatchEvent(new PointerEvent("pointermove", { clientX: x, clientY: y, pointerType: "mouse", bubbles: true }));
    };
    frame = requestAnimationFrame(step);
    let frames = 0;
    const count = () => {
      frames++;
      counter = requestAnimationFrame(count);
    };
    let counter = requestAnimationFrame(count);
    const report = window.setInterval(() => {
      invoke("log", { message: `sweep: ${frames / 2} fps, ${document.visibilityState}, focus ${document.hasFocus()}` }).catch(() => {});
      frames = 0;
    }, 2000);
    return () => {
      cancelAnimationFrame(frame);
      cancelAnimationFrame(counter);
      window.clearInterval(report);
    };
  }, [!!library]);

  const ready = !!library && !!settings;
  const loadError = error ?? libraryError ?? null;
  useEffect(() => (ready || loadError ? dismissBoot() : undefined), [ready, !!loadError]);
  useEffect(() => {
    if (ready) {
      requestAnimationFrame(() =>
        invoke("log", { message: `ready: ${library!.projects.length} tools, video ${canPlayVideo ? "on" : "off"}${location.hash ? `, view ${location.hash}` : ""}` }).catch(() => {}),
      );
    }
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

  const fx = (content: ReactNode) => <EffectsProvider level={settings?.effects ?? "full"}>{content}</EffectsProvider>;

  if (!library || !settings) {
    return fx(
      loadError ? (
        <div className="grid h-full place-items-center">
          <div className="flex flex-col items-center gap-4 text-ink-3">
            <img src={logo} alt="" className="h-12 w-12" />
            <div className="selectable max-w-[60ch] text-center text-[13px]">{loadError}</div>
          </div>
        </div>
      ) : null,
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

  const tabItem = (t: Tab, label: string, icon: string, badge: number, badgeClass: string) => (
    <button key={t} className="nav-item" aria-current={tab === t && !openId && (t !== "browse" || (!filters.cat && !filters.fav))} onClick={() => (t === "browse" ? pickCategory(null) : go(t))}>
      <Icon name={icon} />
      {label}
      {badge > 0 && <span className={`ml-auto rounded-full px-1.5 py-px font-mono text-[10.5px] ${badgeClass}`}>{badge}</span>}
    </button>
  );
  const blocks: Record<Block, ReactNode> = {
    browse: tabItem("browse", "Discover", "grid", 0, ""),
    favorites: (
      <button
        key="favorites"
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
    ),
    assistant: settings.assistantEnabled ? (
      <button key="assistant" className="nav-item" onClick={() => setAssistantOpen(true)}>
        <img src={assistantGlyph} alt="" draggable={false} className="-mx-px h-[18px] w-[18px] shrink-0" /> Assistant
        <span className="ml-auto text-[10.5px] text-ink-3">NEW</span>
      </button>
    ) : null,
    ttz: (
      <button key="ttz" className="nav-item" onClick={() => { setTtzStart(null); setTtzOpen(true); }}>
        <img src={ttzIcon} alt="" draggable={false} className="-mx-px h-[18px] w-[18px] shrink-0 rounded-[5px]" /> Talk to TheZ
        {ttzUnread > 0 ? <span className="ml-auto grid h-[18px] min-w-[18px] place-items-center rounded-full bg-cyan px-1 text-[11px] font-semibold text-[#04131c]" role="img" aria-label={`${ttzUnread} new ${ttzUnread === 1 ? "reply" : "replies"}`}>{ttzUnread}</span> : <span className="ml-auto text-[10.5px] text-ink-3">NEW</span>}
      </button>
    ),
    installed: tabItem("installed", "Library", "box", updates || installed, updates ? "bg-mark text-[#1b1505]" : "bg-line-2 text-ink-2"),
    downloads: tabItem("downloads", "Downloads", "download", active, "bg-cyan text-[#04131c]"),
    settings: tabItem("settings", "Settings", "gear", 0, ""),
    about: tabItem("about", "About", "info", updateWaiting ? 1 : 0, "bg-cyan text-[#04131c]"),
  };
  const sideOrder = blockOrder(settings.sidebar).filter((b) => !blockHidden(settings.sidebarHidden, b));

  const where = project
    ? `${filters.fav ? "Favorites" : (filters.cat ?? "Discover")}  ›  ${project.title}`
    : tab === "browse"
      ? filters.fav
        ? "Favorites"
        : (filters.cat ?? (filters.q ? `Search: ${filters.q}` : "Discover"))
      : { installed: "Library", downloads: "Downloads", settings: "Settings", about: "About", browse: "Discover" }[tab];
  const ownFrame = !settings.systemFrame || library.host.os === "macos";

  const navKey = `${tab}|${openId ?? ""}|${filters.cat ?? ""}|${filters.fav}|${sideOrder.join(",")}`;

  return fx(
    <Sparks>
    <div className={`relative z-10 flex h-full flex-col ${ownFrame && library.host.os !== "macos" ? "frameless" : ""}`}>
      {ownFrame && <TitleBar os={library.host.os} where={where} admin={admin} onAdmin={toggleAdmin} update={updateWaiting ? selfUpdate : null} onUpdate={() => (selfUpdate?.state === "ready" && selfUpdate.canInstall ? api.selfUpdateInstall().catch(() => go("about")) : go("about"))} />}
      {ownFrame && library.host.os !== "macos" && <ResizeEdges />}
      <div className="relative flex min-h-0 flex-1">
      <FxLayer />
      <aside className="fx-side flex w-[248px] shrink-0 flex-col gap-5 border-r border-line px-3.5 pt-4 pb-4">

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

        <nav className="nav-main relative flex flex-col gap-0.5">
          <NavIndicator active={navKey} />
          {sideOrder.map((b) => blocks[b])}
        </nav>

        <div className="scroll -mx-1 flex min-h-0 flex-1 flex-col gap-0.5 px-1">
          <div className="eyebrow px-2.5 pb-1.5">Categories</div>
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
      {ttzOpen && <TalkToZ key={ttzStart ?? "list"} start={ttzStart} close={() => { setTtzOpen(false); checkTtz(); }} titleBar={ownFrame} mac={library.host.os === "macos"} />}
      {ttzReply && (
        <div className="toast fixed right-5 bottom-5 z-[90] flex w-[340px] items-start gap-3 rounded-2xl p-3.5">
          <img src={ttzIcon} alt="" className="h-8 w-8 shrink-0 rounded-full" />
          <button className="min-w-0 flex-1 text-left" onClick={() => { setTtzStart(ttzReply.id); setTtzOpen(true); setTtzReply(null); }}>
            <div className="font-semibold">TheHolyOneZ replied</div>
            <div className="line-clamp-2 text-[12.5px] text-ink-2">{ttzReply.text}</div>
          </button>
          <button className="text-ink-3 hover:text-ink" title="Dismiss" onClick={() => setTtzReply(null)}><Icon name="x" /></button>
        </div>
      )}
      {assistantOpen && <Assistant lib={library} jobs={jobs} close={() => setAssistantOpen(false)} openProject={(id) => { setAssistantOpen(false); go("browse"); setOpenId(id); }} openSettings={() => { setAssistantOpen(false); go("settings"); }} openChat={(id) => { setAssistantOpen(false); setTtzStart(id); setTtzOpen(true); }} admin={admin} requestAdmin={requestAssistantAdmin} settings={settings} save={save} titleBar={ownFrame} />}
      <Toasts jobs={jobs} open={(id: string) => setOpenId(id)} onDismiss={(j: Job) => clear(j.projectId)} />
      <Overlays />
    </div>
    </Sparks>,
  );
}
