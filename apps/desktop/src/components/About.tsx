import { ReactNode, useMemo, useState } from "react";
import { api, bytes, Library, OS_LABEL, osesOf, Project } from "../api";
import { Icon } from "./Icon";
import { OsIcon } from "./OsIcon";

const LICENSE = /^(A?GPL|LGPL|MIT|Apache|BSD|MPL|ISC|Unlicense|CC|Proprietary|EUPL|Zlib)/i;

export function sentences(text: string): string[] {
  const out: string[] = [];
  let cur = "";
  const parts = text.split(/(?<=[.!?])\s+(?=[A-Z0-9"“(])/);
  for (const p of parts) {
    cur = cur ? `${cur} ${p}` : p;
    if (/\b(e\.g|i\.e|etc|vs|incl|approx)\.$/i.test(cur)) continue;
    out.push(cur.trim());
    cur = "";
  }
  if (cur.trim()) out.push(cur.trim());
  return out;
}

export function techLine(all: string[]): { tags: string[]; license: string | null; rest: string[] } {
  const last = all[all.length - 1] ?? "";
  const bits = last.replace(/\.$/, "").split(/\s*·\s*/);
  if (bits.length < 2 || bits.some((b) => b.length > 32 || b.split(" ").length > 4)) {
    return { tags: [], license: null, rest: all };
  }
  const license = bits.find((b) => LICENSE.test(b)) ?? null;
  return { tags: bits.filter((b) => b !== license), license, rest: all.slice(0, -1) };
}

function paragraphs(list: string[]): string[] {
  const out: string[] = [];
  let cur = "";
  for (const s of list) {
    if (cur && cur.length + s.length > 330) {
      out.push(cur);
      cur = s;
    } else {
      cur = cur ? `${cur} ${s}` : s;
    }
  }
  if (cur) out.push(cur);
  return out;
}

function Fact({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex items-start justify-between gap-4 py-2.5">
      <span className="shrink-0 text-[13px] text-ink-3">{label}</span>
      <span className="min-w-0 text-right text-[13.5px] font-medium text-ink">{children}</span>
    </div>
  );
}

function when(d: string | null): string | null {
  if (!d) return null;
  const t = Date.parse(d);
  return Number.isNaN(t) ? d : new Date(t).toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric" });
}

export function About({ p, lib }: { p: Project; lib: Library }) {
  const [more, setMore] = useState(false);
  const { paras, tags, license } = useMemo(() => {
    const all = sentences(p.description);
    const { tags, license, rest } = techLine(all);
    const body = (rest.length ? rest : all).slice(1);
    return { paras: paragraphs(body), tags, license };
  }, [p.description]);
  const long = paras.join(" ").length > 650;
  const shown = long && !more ? paras.slice(0, 1) : paras;
  const rec = p.artifacts.find((a) => a.url === p.recommended);
  const oses = osesOf(p);
  const source = p.links.find((l) => l.url.includes("github.com"));

  return (
    <section className="grid items-start gap-6 lg:grid-cols-[1fr_320px]">
      <div className="card relative overflow-hidden p-7">
        <div className="glow -top-28 -right-28 h-80 w-80 [--glow:color-mix(in_srgb,var(--violet)_14%,transparent)]" />
        <div className="mb-3 text-[12px] font-semibold tracking-[.14em] text-ink-3 uppercase">About</div>
        {shown.length === 0 && <p className="text-[15px] text-ink-3">That's all there is to say about this one; the facts are on the right.</p>}
        {shown.length > 0 && (
          <div className={`selectable relative flex flex-col gap-4 text-[15.5px] leading-[1.75] text-ink-2 ${long && !more ? "max-h-[11.5em] overflow-hidden" : ""}`}>
            {shown.map((t, i) => (
              <p key={i} className={`max-w-[68ch] ${i === 0 ? "text-ink" : ""}`}>
                {t}
              </p>
            ))}
            {long && !more && <div className="absolute inset-x-0 bottom-0 h-16 bg-gradient-to-t from-card to-transparent" />}
          </div>
        )}
        {long && (
          <button className="relative mt-3 flex items-center gap-1 text-[13.5px] font-medium text-cyan hover:underline" onClick={() => setMore(!more)}>
            {more ? "Show less" : "Read more"}
            <Icon name="back" className={`!h-3.5 !w-3.5 ${more ? "rotate-90" : "-rotate-90"}`} />
          </button>
        )}
        {tags.length > 0 && (
          <div className="relative mt-6 flex flex-wrap items-center gap-1.5 border-t border-line pt-5">
            <span className="mr-1 text-[12px] font-semibold tracking-[.12em] text-ink-3 uppercase">Built with</span>
            {tags.map((t) => (
              <span key={t} className="rounded-lg border border-line bg-bg-2 px-2.5 py-1 text-[12.5px] font-medium text-ink-2">
                {t}
              </span>
            ))}
          </div>
        )}
      </div>

      <aside className="card divide-y divide-line px-5 py-2">
        {p.version && <Fact label="Version">v{p.version}</Fact>}
        {when(p.released) && <Fact label="Released">{when(p.released)}</Fact>}
        {oses.length > 0 && (
          <Fact label="Runs on">
            <span className="flex flex-wrap justify-end gap-x-2.5 gap-y-1">
              {oses.map((o) => (
                <span key={o} className="flex items-center gap-1">
                  <OsIcon os={o} here={o === lib.host.os} size={13} /> {OS_LABEL[o]}
                </span>
              ))}
            </span>
          </Fact>
        )}
        {rec?.size && <Fact label="Download">{bytes(rec.size)}</Fact>}
        {license && <Fact label="License">{license}</Fact>}
        {p.category && <Fact label="Category">{p.category}</Fact>}
        {p.added && <Fact label="On zsync since">{when(p.added)}</Fact>}
        {source && (
          <Fact label="Source">
            <button className="flex items-center gap-1 text-cyan hover:underline" onClick={() => api.openUrl(source.url)}>
              GitHub <Icon name="link" className="!h-3.5 !w-3.5" />
            </button>
          </Fact>
        )}
      </aside>
    </section>
  );
}
