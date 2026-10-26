import { Fragment, ReactNode } from "react";
import { api } from "../api";
import { fail } from "./Overlays";

const INLINE = /(`[^`\n]+`)|(\*\*[^*\n]+?\*\*|__[^_\n]+?__)|(~~[^~\n]+?~~)|(\[[^\]\n]+\]\([^)\s]+\))|(https?:\/\/[^\s)<>]+[^\s)<>.,;:!?'"])|(\*[^*\s][^*\n]*?\*)/;

function openLink(url: string) {
  if (/^https?:\/\//.test(url)) api.openUrl(url).catch(fail);
}

const prose = (text: string) => text.replace(/ - /g, " – ");

function inline(text: string, key = "i"): ReactNode[] {
  const out: ReactNode[] = [];
  let rest = text;
  let n = 0;
  while (rest) {
    const m = INLINE.exec(rest);
    if (!m) {
      out.push(prose(rest));
      break;
    }
    if (m.index > 0) out.push(prose(rest.slice(0, m.index)));
    const t = m[0];
    const k = `${key}-${n++}`;
    if (m[1]) out.push(<code key={k} className="md-code">{t.slice(1, -1)}</code>);
    else if (m[2]) out.push(<strong key={k}>{inline(t.slice(2, -2), k)}</strong>);
    else if (m[3]) out.push(<del key={k}>{inline(t.slice(2, -2), k)}</del>);
    else if (m[4]) {
      const [, label, url] = /^\[([^\]]+)\]\(([^)\s]+)\)$/.exec(t) ?? [];
      out.push(<a key={k} href={url} title={url} onClick={(e) => (e.preventDefault(), openLink(url))}>{inline(label, k)}</a>);
    } else if (m[5]) out.push(<a key={k} href={t} onClick={(e) => (e.preventDefault(), openLink(t))}>{t.replace(/^https?:\/\//, "")}</a>);
    else if (m[6]) out.push(<em key={k}>{inline(t.slice(1, -1), k)}</em>);
    rest = rest.slice(m.index + t.length);
  }
  return out;
}

function joined(text: string[], key: string): ReactNode {
  return inline(text.join(" "), key);
}

function lines(text: string[], key: string): ReactNode {
  return text.map((l, i) => <Fragment key={`${key}-${i}`}>{i > 0 && <br />}{inline(l, `${key}-${i}`)}</Fragment>);
}

type Block =
  | { t: "p"; lines: string[] }
  | { t: "h"; level: number; text: string }
  | { t: "code"; lang: string; text: string }
  | { t: "list"; ordered: boolean; start: number; items: { text: string[]; sub: string[] }[] }
  | { t: "quote"; lines: string[] }
  | { t: "table"; head: string[]; rows: string[][] }
  | { t: "hr" };

const LIST = /^(\s*)([-*+•]|\d{1,3}[.)])\s+(.*)$/;
const cells = (row: string) => row.trim().replace(/^\|/, "").replace(/\|$/, "").split("|").map((c) => c.trim());

function parse(src: string): Block[] {
  const rows = src.replace(/\r/g, "").split("\n");
  const blocks: Block[] = [];
  let i = 0;
  while (i < rows.length) {
    const row = rows[i];
    if (!row.trim()) {
      i++;
      continue;
    }
    const fence = /^\s*(```|~~~)\s*([\w+-]*)/.exec(row);
    if (fence) {
      const body: string[] = [];
      i++;
      while (i < rows.length && !rows[i].trim().startsWith(fence[1])) body.push(rows[i++]);
      i++;
      blocks.push({ t: "code", lang: fence[2], text: body.join("\n") });
      continue;
    }
    const heading = /^\s{0,3}(#{1,6})\s+(.*?)\s*#*\s*$/.exec(row);
    if (heading) {
      blocks.push({ t: "h", level: heading[1].length, text: heading[2] });
      i++;
      continue;
    }
    if (/^\s{0,3}([-*_])(\s*\1){2,}\s*$/.test(row)) {
      blocks.push({ t: "hr" });
      i++;
      continue;
    }
    if (row.includes("|") && i + 1 < rows.length && /^\s*\|?\s*:?-{2,}:?\s*(\|\s*:?-{2,}:?\s*)*\|?\s*$/.test(rows[i + 1])) {
      const head = cells(row);
      const body: string[][] = [];
      i += 2;
      while (i < rows.length && rows[i].includes("|") && rows[i].trim()) body.push(cells(rows[i++]));
      blocks.push({ t: "table", head, rows: body });
      continue;
    }
    if (/^\s*>/.test(row)) {
      const quote: string[] = [];
      while (i < rows.length && /^\s*>/.test(rows[i])) quote.push(rows[i++].replace(/^\s*>\s?/, ""));
      blocks.push({ t: "quote", lines: quote });
      continue;
    }
    const item = LIST.exec(row);
    if (item) {
      const ordered = /\d/.test(item[2]);
      const base = item[1].length;
      const list: Block & { t: "list" } = { t: "list", ordered, start: ordered ? parseInt(item[2], 10) : 1, items: [] };
      while (i < rows.length) {
        const r = rows[i];
        const m = LIST.exec(r);
        if (m && m[1].length <= base + 1 && /\d/.test(m[2]) === ordered) {
          list.items.push({ text: [m[3]], sub: [] });
          i++;
        } else if (m && m[1].length > base + 1 && list.items.length) {
          list.items[list.items.length - 1].sub.push(m[3]);
          i++;
        } else if (r.trim() && /^\s+/.test(r) && list.items.length) {
          list.items[list.items.length - 1].text.push(r.trim());
          i++;
        } else if (!r.trim() && i + 1 < rows.length && LIST.exec(rows[i + 1]) && /\d/.test(LIST.exec(rows[i + 1])![2]) === ordered) {
          i++;
        } else break;
      }
      blocks.push(list);
      continue;
    }
    const para: string[] = [];
    while (i < rows.length && rows[i].trim() && !LIST.exec(rows[i]) && !/^\s*(```|~~~|#{1,6}\s|>)/.test(rows[i])) {
      const line = rows[i++].trim();
      para.push(line);
      if (/[.!?:]["')\]*_]*$/.test(line)) break;
    }
    if (!para.length) para.push(rows[i++].trim());
    blocks.push({ t: "p", lines: para });
  }
  return blocks;
}

export function Markdown({ text, streaming = false, className = "" }: { text: string; streaming?: boolean; className?: string }) {
  const blocks = parse(text);
  const caret = streaming ? <span className="md-caret" aria-hidden="true" /> : null;
  return (
    <div className={`md ${className}`}>
      {blocks.map((b, n) => {
        const k = `b${n}`;
        const last = n === blocks.length - 1;
        switch (b.t) {
          case "p":
            return <p key={k}>{joined(b.lines, k)}{last && caret}</p>;
          case "h":
            return b.level <= 2 ? <h3 key={k}>{inline(b.text, k)}</h3> : <h4 key={k}>{inline(b.text, k)}</h4>;
          case "code":
            return <pre key={k} className="md-pre"><code>{b.text}</code>{last && caret}</pre>;
          case "hr":
            return <hr key={k} />;
          case "quote":
            return <blockquote key={k}>{lines(b.lines, k)}</blockquote>;
          case "table":
            return (
              <div key={k} className="md-table">
                <table>
                  <thead><tr>{b.head.map((c, j) => <th key={j}>{inline(c, `${k}h${j}`)}</th>)}</tr></thead>
                  <tbody>{b.rows.map((r, j) => <tr key={j}>{r.map((c, x) => <td key={x}>{inline(c, `${k}r${j}${x}`)}</td>)}</tr>)}</tbody>
                </table>
              </div>
            );
          case "list": {
            const items = b.items.map((it, j) => (
              <li key={j}>
                {joined(it.text, `${k}l${j}`)}
                {it.sub.length > 0 && <ul>{it.sub.map((s, x) => <li key={x}>{inline(s, `${k}l${j}s${x}`)}</li>)}</ul>}
                {last && j === b.items.length - 1 && caret}
              </li>
            ));
            return b.ordered ? <ol key={k} start={b.start}>{items}</ol> : <ul key={k}>{items}</ul>;
          }
        }
      })}
      {blocks.length === 0 && caret}
    </div>
  );
}
