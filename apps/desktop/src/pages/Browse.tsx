import { memo, useDeferredValue, useEffect, useMemo, useRef, useState } from "react";
import { Job, Library, Os, osesOf, Project, summary } from "../api";
import { Action } from "../components/Action";
import { Select } from "../components/Controls";
import { Hero } from "../components/Hero";
import { Icon } from "../components/Icon";
import { Media } from "../components/Media";
import { OsIcon } from "../components/OsIcon";
import { hideVideo } from "../components/videoPool";
import { untilPainted } from "../perf";
import { menu } from "../components/Overlays";
import { projectMenu } from "../components/projectMenu";

export type OsFilter = "all" | "here" | Os;

export interface Filters {
  q: string;
  os: OsFilter;
  cat: string | null;
  retired: boolean;
  fav: boolean;
}

export type Sort = "newest" | "name" | "here";

export function Heart({ on, toggle, className = "" }: { on: boolean; toggle: () => void; className?: string }) {
  return (
    <button
      className={`grid h-8 w-8 place-items-center rounded-full transition-all ${on ? "text-[#ff5d8f]" : "text-ink-3 hover:text-ink"} ${className}`}
      title={on ? "Remove from favorites" : "Add to favorites"}
      aria-pressed={on}
      onClick={(e) => {
        e.stopPropagation();
        toggle();
      }}
    >
      <svg viewBox="0 0 24 24" className={`h-[18px] w-[18px] transition-transform ${on ? "scale-110" : ""}`} fill={on ? "currentColor" : "none"} stroke="currentColor" strokeWidth={1.8} strokeLinejoin="round">
        <path d="M12 20.5s-7.5-4.6-9.3-9.2C1.4 7.9 3.5 4.5 7 4.5c2 0 3.6 1.1 5 2.9 1.4-1.8 3-2.9 5-2.9 3.5 0 5.6 3.4 4.3 6.8-1.8 4.6-9.3 9.2-9.3 9.2z" />
      </svg>
    </button>
  );
}

function sameCard(a: CardProps, b: CardProps): boolean {
  const pa = a.p,
    pb = b.p;
  return (
    pa.id === pb.id &&
    pa.title === pb.title &&
    pa.retired === pb.retired &&
    pa.version === pb.version &&
    pa.recommended === pb.recommended &&
    pa.crawledAt === pb.crawledAt &&
    pa.video?.poster === pb.video?.poster &&
    pa.installed?.version === pb.installed?.version &&
    pa.installed?.update === pb.installed?.update &&
    pa.installed?.present === pb.installed?.present &&
    pa.artifacts.length === pb.artifacts.length &&
    a.lib.host.os === b.lib.host.os &&
    a.job === b.job &&
    a.hover === b.hover &&
    a.fav === b.fav &&
    a.onOpen === b.onOpen &&
    a.onFav === b.onFav
  );
}

interface CardProps {
  p: Project;
  lib: Library;
  job?: Job;
  hover: boolean;
  onOpen: (id: string) => void;
  fav: boolean;
  onFav: (id: string) => void;
}

const HOVER_INTENT_MS = 300;

const Card = memo(function Card({
  p,
  lib,
  job,
  hover,
  onOpen,
  fav,
  onFav,
}: CardProps) {
  const [over, setOver] = useState(false);
  const timer = useRef<number | undefined>(undefined);
  useEffect(() => () => window.clearTimeout(timer.current), []);
  const open = () => onOpen(p.id);
  const toggleFav = () => onFav(p.id);
  const oses = osesOf(p);
  const inst = p.installed?.present ? p.installed : null;
  return (
    <div
      role="button"
      tabIndex={0}
      className="tool group cursor-pointer outline-none"
      onClick={open}
      onKeyDown={(e) => e.key === "Enter" && open()}
      onContextMenu={(e) => menu(e, projectMenu(p, lib, job, { open, fav, toggleFav }))}
      onMouseEnter={() => {
        window.clearTimeout(timer.current);
        if (hover && p.video) timer.current = window.setTimeout(() => setOver(true), HOVER_INTENT_MS);
      }}
      onMouseLeave={() => {
        window.clearTimeout(timer.current);
        setOver(false);
      }}
    >
      <div className="relative">
        <Media p={p} playing={hover && over} className="aspect-[16/9]" />
        <div className="pointer-events-none absolute inset-x-0 bottom-0 h-16 bg-gradient-to-t from-card to-transparent" />
        <div className="absolute top-2.5 left-2.5 flex gap-1.5">
          {inst?.update ? (
            <span className="rounded-md bg-mark px-2 py-0.5 text-[11px] font-bold text-[#1b1505]">Update {inst.update}</span>
          ) : inst ? (
            <span className="flex items-center gap-1 rounded-md bg-black/70 px-2 py-0.5 text-[11px] font-semibold text-ok">
              <Icon name="check" className="!h-3 !w-3" /> Installed
            </span>
          ) : null}
          {p.retired && <span className="rounded-md bg-black/70 px-2 py-0.5 text-[11px] text-white/80">Retired</span>}
        </div>
        {p.version && (
          <span className="absolute top-2.5 right-2.5 rounded-md bg-black/70 px-1.5 py-0.5 font-mono text-[10.5px] text-white/85">
            v{p.version}
          </span>
        )}
      </div>
      <div className="flex flex-1 flex-col gap-1.5 px-4 pt-1 pb-4">
        <div className="flex items-center gap-2">
          <h3 className="truncate text-[17px] font-bold">{p.title}</h3>
          <div className="ml-auto flex shrink-0 items-center gap-1.5">
            {oses.map((o) => (
              <OsIcon key={o} os={o} here={o === lib.host.os} size={14} />
            ))}
          </div>
        </div>
        <p className="line-clamp-2 min-h-[42px] text-[13.5px] leading-[1.55] text-ink-2">{summary(p)}</p>
        <div className="mt-2 flex h-[32px] items-center gap-2">
          <Action p={p} lib={lib} job={job} compact />
          <Heart on={fav} toggle={toggleFav} className={`ml-auto shrink-0 ${fav ? "" : "opacity-0 group-hover:opacity-100 focus-visible:opacity-100"}`} />
        </div>
      </div>
    </div>
  );
}, sameCard);

function Spinner() {
  return <span aria-label="Updating" className="ml-2.5 inline-block h-[15px] w-[15px] animate-spin rounded-full border-2 border-line-2 border-t-cyan align-middle" />;
}

function SkeletonCard() {
  return (
    <div className="tool">
      <div className="skeleton aspect-[16/9]" />
      <div className="flex flex-col gap-2 p-4">
        <div className="skeleton h-4 w-1/2 rounded" />
        <div className="skeleton h-3 w-full rounded" />
        <div className="skeleton h-3 w-3/4 rounded" />
      </div>
    </div>
  );
}

export function filterProjects(lib: Library, f: Filters, favorites: string[] = [], sort: Sort = "newest"): Project[] {
  const words = f.q.toLowerCase().split(/\s+/).filter(Boolean);
  const order: Record<Sort, (a: Project, b: Project) => number> = {
    newest: (a, b) => (b.added ?? "").localeCompare(a.added ?? ""),
    name: (a, b) => a.title.localeCompare(b.title, undefined, { sensitivity: "base" }),
    here: (a, b) => Number(!!b.recommended) - Number(!!a.recommended) || (b.added ?? "").localeCompare(a.added ?? ""),
  };
  return lib.projects
    .filter((p) => !f.fav || favorites.includes(p.id))
    .filter((p) => f.retired || !p.retired || words.length > 0 || f.fav)
    .filter((p) => !f.cat || p.category === f.cat || p.categories.includes(f.cat))
    .filter((p) => {
      if (f.os === "all") return true;
      if (f.os === "here") return !!p.recommended;
      return p.artifacts.some((a) => a.os === f.os);
    })
    .filter((p) => {
      const hay = `${p.title} ${p.description} ${p.category ?? ""} ${p.categories.join(" ")}`.toLowerCase();
      return words.every((w) => hay.includes(w));
    })
    .sort(order[sort]);
}

export function Browse({
  active,
  lib,
  jobs,
  hover,
  filters,
  setFilters,
  open,
  favorites,
  toggleFav,
  sort,
  setSort,
}: {
  active: boolean;
  lib: Library;
  jobs: Record<string, Job>;
  hover: boolean;
  filters: Filters;
  setFilters: (f: Filters) => void;
  open: (id: string) => void;
  favorites: string[];
  toggleFav: (id: string) => void;
  sort: Sort;
  setSort: (s: Sort) => void;
}) {
  const view = useDeferredValue(filters);
  const viewSort = useDeferredValue(sort);
  const updating = view !== filters || viewSort !== sort;
  const shown = useMemo(() => filterProjects(lib, view, favorites, viewSort), [lib, view, favorites, viewSort]);
  const position = useMemo(() => new Map(shown.map((p, i) => [p.id, i])), [shown]);
  useEffect(() => {
    if (!active) hideVideo("card");
    untilPainted(active ? "discover shown" : "discover hidden");
  }, [active]);
  const searching = view.q.trim() !== "" || view.cat !== null || view.os !== "all" || view.fav;
  const featured = useMemo(
    () => [...lib.projects].filter((p) => p.video && !p.retired).sort((a, b) => (b.added ?? "").localeCompare(a.added ?? "")).slice(0, 6),
    [lib.projects],
  );
  const firstCrawl = lib.crawling !== null && lib.projects.every((p) => p.crawledAt === null);
  const set = (patch: Partial<Filters>) => setFilters({ ...filters, ...patch });

  const here = lib.host.os === "macos" ? "Mac" : "PC";
  const osChoices: [OsFilter, string][] = [
    ["all", "Everything"],
    ["here", `Runs on this ${here}`],
    ["windows", "Windows"],
    ["linux", "Linux"],
    ["macos", "macOS"],
    ["android", "Android"],
  ];

  return (
    <div className="page mx-auto flex max-w-[1320px] flex-col gap-7 px-8 py-7">
      {!searching && featured.length > 0 && <Hero items={featured} lib={lib} jobs={jobs} open={open} active={active} />}

      <div className="flex flex-wrap items-center gap-3">
        {filters.fav ? (
          <div className="flex items-center gap-3.5">
            <span className="grid h-11 w-11 place-items-center rounded-2xl bg-[#ff5d8f]/12 text-[#ff5d8f] ring-1 ring-[#ff5d8f]/25">
              <svg viewBox="0 0 24 24" className="h-5 w-5" fill="currentColor">
                <path d="M12 20.5s-7.5-4.6-9.3-9.2C1.4 7.9 3.5 4.5 7 4.5c2 0 3.6 1.1 5 2.9 1.4-1.8 3-2.9 5-2.9 3.5 0 5.6 3.4 4.3 6.8-1.8 4.6-9.3 9.2-9.3 9.2z" />
              </svg>
            </span>
            <div>
              <h1 className="text-[28px] leading-tight font-bold">
                Favorites <span className="align-middle font-mono text-[13px] font-normal text-ink-3">{shown.length}</span>
                {updating && <Spinner />}
              </h1>
              <p className="text-[13.5px] text-ink-3">The tools you starred, in one place.</p>
            </div>
          </div>
        ) : (
          <h1 className="text-[26px] font-bold">
            {filters.cat ?? (filters.q ? `“${filters.q}”` : "All tools")}
            <span className="ml-2.5 align-middle font-mono text-[13px] font-normal text-ink-3">{shown.length}</span>
            {updating && <Spinner />}
          </h1>
        )}
        <div className="ml-auto flex flex-wrap items-center gap-1.5">
          <Select
            size="sm"
            label="Sort"
            className="mr-1 w-[168px]"
            value={sort}
            onChange={setSort}
            options={[
              { value: "newest", label: "Newest first", icon: <Icon name="sort" className="text-ink-3" /> },
              { value: "name", label: "A–Z", icon: <Icon name="sort" className="text-ink-3" /> },
              { value: "here", label: "Installable first", icon: <Icon name="sort" className="text-ink-3" /> },
            ]}
          />
          {osChoices.map(([k, label]) => (
            <button key={k} className="chip" aria-pressed={filters.os === k} onClick={() => set({ os: k })}>
              {k !== "all" && k !== "here" && <OsIcon os={k} size={12} here={filters.os === k} />}
              {label}
            </button>
          ))}
        </div>
      </div>

      {firstCrawl ? (
        <div className="grid gap-5 [grid-template-columns:repeat(auto-fill,minmax(280px,1fr))]">
          {Array.from({ length: 8 }, (_, n) => (
            <SkeletonCard key={n} />
          ))}
        </div>
      ) : shown.length === 0 && view.fav ? (
        <div className="flex flex-col items-center gap-3 py-20 text-center text-ink-3">
          <Icon name="heart" className="!h-8 !w-8" />
          <p>No favorites yet. Tap the heart on any tool to keep it here.</p>
        </div>
      ) : shown.length === 0 && !updating ? (
        <div className="flex flex-col items-center gap-3 py-20 text-center text-ink-3">
          <Icon name="search" className="!h-8 !w-8" />
          <p>Nothing matches that. Try fewer words or another filter.</p>
          <button className="btn sm" onClick={() => setFilters({ q: "", os: "all", cat: null, retired: false, fav: false })}>
            Clear filters
          </button>
        </div>
      ) : null}
      {!firstCrawl && (
        <div
          className={`grid gap-5 transition-opacity duration-150 [grid-template-columns:repeat(auto-fill,minmax(280px,1fr))] ${updating ? "opacity-60" : ""}`}
          hidden={shown.length === 0}
        >
          {lib.projects.map((p) => {
            const at = position.get(p.id);
            return (
              <div key={p.id} hidden={at === undefined} style={{ order: at }} className="flex flex-col [&>*]:flex-1">
                <Card p={p} lib={lib} job={jobs[p.id]} hover={hover} onOpen={open} fav={favorites.includes(p.id)} onFav={toggleFav} />
              </div>
            );
          })}
        </div>
      )}

      {!filters.retired && lib.projects.some((p) => p.retired) && !searching && (
        <button className="self-center text-[13px] text-ink-3 hover:text-ink" onClick={() => set({ retired: true })}>
          Show retired tools too
        </button>
      )}
    </div>
  );
}
