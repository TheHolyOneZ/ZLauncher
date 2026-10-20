import { ReactNode, useState } from "react";

function inline(text: string): ReactNode[] {
  const out: ReactNode[] = [];
  const re = /(\*\*[^*]+\*\*|`[^`]+`)/g;
  let last = 0;
  let m: RegExpExecArray | null;
  while ((m = re.exec(text))) {
    if (m.index > last) out.push(text.slice(last, m.index));
    const t = m[0];
    out.push(
      t.startsWith("**") ? (
        <strong key={m.index} className="font-semibold text-ink">
          {t.slice(2, -2)}
        </strong>
      ) : (
        <code key={m.index} className="rounded bg-bg-2 px-1 font-mono text-[12px]">
          {t.slice(1, -1)}
        </code>
      ),
    );
    last = m.index + t.length;
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}

function render(md: string): ReactNode[] {
  const blocks: ReactNode[] = [];
  let list: string[] = [];
  const flush = () => {
    if (list.length) {
      blocks.push(
        <ul key={`l${blocks.length}`} className="flex list-disc flex-col gap-1 pl-5 marker:text-cyan">
          {list.map((li, i) => (
            <li key={i}>{inline(li)}</li>
          ))}
        </ul>,
      );
      list = [];
    }
  };
  for (const raw of md.replace(/\r/g, "").split("\n")) {
    const line = raw.trimEnd();
    const bullet = line.match(/^\s*[-*+]\s+(.*)$/);
    const heading = line.match(/^#{1,6}\s+(.*)$/);
    if (bullet) {
      list.push(bullet[1]);
    } else {
      flush();
      if (heading) {
        blocks.push(
          <div key={blocks.length} className="pt-1 font-semibold text-ink">
            {inline(heading[1])}
          </div>,
        );
      } else if (line.trim()) {
        blocks.push(<p key={blocks.length}>{inline(line)}</p>);
      }
    }
  }
  flush();
  return blocks;
}

function when(d: string | null): string | null {
  if (!d) return null;
  const t = Date.parse(d);
  return Number.isNaN(t) ? d : new Date(t).toLocaleDateString(undefined, { year: "numeric", month: "long", day: "numeric" });
}

export function WhatsNew({ notes, version, released }: { notes: string | null; version: string | null; released: string | null }) {
  const [open, setOpen] = useState(false);
  if (!notes) return null;
  const long = notes.length > 420 || notes.split("\n").length > 8;
  return (
    <section className="card relative overflow-hidden p-5">
      <div className="glow -top-24 -right-24 h-72 w-72 [--glow:color-mix(in_srgb,var(--cyan)_14%,transparent)]" />
      <div className="mb-3 flex items-baseline gap-3">
        <h3 className="text-[17px] font-bold">What's new{version ? ` in ${version}` : ""}</h3>
        {when(released) && <span className="text-[12.5px] text-ink-3">{when(released)}</span>}
      </div>
      <div className={`selectable relative flex flex-col gap-2 text-[14px] leading-[1.65] text-ink-2 ${long && !open ? "max-h-40 overflow-hidden" : ""}`}>
        {render(notes)}
        {long && !open && <div className="absolute inset-x-0 bottom-0 h-14 bg-gradient-to-t from-card to-transparent" />}
      </div>
      {long && (
        <button className="mt-2 text-[13px] text-cyan hover:underline" onClick={() => setOpen(!open)}>
          {open ? "Show less" : "Show all"}
        </button>
      )}
    </section>
  );
}
