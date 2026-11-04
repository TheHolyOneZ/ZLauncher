import { memo, ReactNode, useState } from "react";
import { api } from "../api";
import { fail } from "./Overlays";

const MAX_DEPTH = 10;
const MAX_GAP = 6;
const ESCAPABLE = "\\`*_{}[]()#+-.!|~>";
const SAFE_PROTOCOLS = ["http:", "https:", "mailto:"];
const RE_FENCE = /^ {0,3}(`{3,}|~{3,})\s*([\w+#.-]*)[^`]*$/;
const RE_HR = /^ {0,3}([-*_])(?:[ \t]*\1){2,}[ \t]*$/;
const RE_HEADING = /^ {0,3}(#{1,3})[ \t]+(.+?)[ \t#]*$/;
const RE_QUOTE = /^ {0,3}> ?/;
const RE_LIST = /^([ \t]*)([-*+•]|\d{1,9}[.)])[ \t]+(.*)$/;
const RE_TABLE_SEP = /^\s*\|?\s*:?-+:?\s*(\|\s*:?-+:?\s*)*\|?\s*$/;

const isWordChar = (c?: string) => !!c && /[\p{L}\p{N}_]/u.test(c);
const isSpace = (c?: string) => !c || /\s/.test(c);

function indentOf(s: string) {
  let n = 0;
  for (const c of s) {
    if (c === " ") n++;
    else if (c === "\t") n += 4;
    else break;
  }
  return n;
}

function dedent(line: string, n: number) {
  let i = 0;
  let w = 0;
  while (i < line.length && w < n) {
    if (line[i] === " ") w++;
    else if (line[i] === "\t") w += 4;
    else break;
    i++;
  }
  return line.slice(i);
}

function safeUrl(raw: string) {
  let s = raw.trim();
  if (!s) return null;
  if (/^www\./i.test(s)) s = `https://${s}`;
  if (/^[\w.+-]+@[\w-]+(\.[\w-]+)+$/.test(s)) s = `mailto:${s}`;
  try {
    const u = new URL(s);
    return SAFE_PROTOCOLS.includes(u.protocol) ? u.href : null;
  } catch {
    return null;
  }
}

function hostOf(href: string) {
  try {
    const u = new URL(href);
    return u.protocol === "mailto:" ? href.slice(7) : u.host;
  } catch {
    return href;
  }
}

function Link({ href, auto, image, children }: { href: string; auto?: boolean; image?: boolean; children: ReactNode }) {
  return (
    <a className={`cm-link ${image ? "cm-imglink" : ""}`} href={href} title={href} data-host={auto ? undefined : hostOf(href)} onClick={(e) => { e.preventDefault(); api.openUrl(href).catch(fail); }}>
      {children}
    </a>
  );
}

function Spoiler({ children }: { children: ReactNode }) {
  const [shown, setShown] = useState(false);
  return (
    <span className={`cm-spoiler ${shown ? "shown" : ""}`} role="button" tabIndex={0} aria-label="Spoiler, activate to reveal" aria-pressed={shown}
      onClick={() => setShown((s) => !s)} onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); setShown((s) => !s); } }}>
      {children}
    </span>
  );
}

export async function copyToClipboard(text: string) {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    const area = document.createElement("textarea");
    area.value = text;
    area.style.position = "fixed";
    area.style.opacity = "0";
    document.body.appendChild(area);
    area.select();
    document.execCommand("copy");
    area.remove();
  }
}

function CodeBlock({ lang, body }: { lang: string; body: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <div className="cm-pre">
      <div className="cm-pre-head">
        <span>{lang || "code"}</span>
        <button type="button" aria-label="Copy code" onClick={() => void copyToClipboard(body).then(() => { setCopied(true); window.setTimeout(() => setCopied(false), 1400); })}>{copied ? "Copied" : "Copy"}</button>
      </div>
      <pre className="selectable"><code>{body}</code></pre>
    </div>
  );
}

function findPairClose(src: string, delim: string, from: number) {
  const L = delim.length;
  let idx = src.indexOf(delim, from);
  while (idx !== -1) {
    if (idx > from && !isSpace(src[idx - 1])) {
      if (!(L === 2 && src[idx + 2] === delim[0] && src.indexOf(delim, idx + 1) === idx + 1)) return idx;
    }
    idx = src.indexOf(delim, idx + 1);
  }
  return -1;
}

function findSingleClose(src: string, ch: string, from: number) {
  let idx = src.indexOf(ch, from);
  while (idx !== -1) {
    if (src[idx + 1] === ch) {
      let run = idx;
      while (src[run] === ch) run++;
      if ((run - idx) % 2 === 1 && !isSpace(src[idx - 1]) && idx > from) return run - 1;
      idx = src.indexOf(ch, run);
      continue;
    }
    if (idx > from && !isSpace(src[idx - 1]) && (ch !== "_" || !isWordChar(src[idx + 1]))) return idx;
    idx = src.indexOf(ch, idx + 1);
  }
  return -1;
}

function parseLinkAt(src: string, i: number) {
  let depth = 0;
  let j = i;
  for (; j < src.length; j++) {
    const c = src[j];
    if (c === "\\") { j++; continue; }
    if (c === "\n" && src[j + 1] === "\n") return null;
    if (c === "[") depth++;
    else if (c === "]") { depth--; if (depth === 0) break; }
  }
  if (j >= src.length || src[j + 1] !== "(") return null;
  const text = src.slice(i + 1, j);
  let k = j + 2;
  let pd = 1;
  for (; k < src.length; k++) {
    const c = src[k];
    if (c === "\n" || (c === " " && pd === 1 && src[k + 1] !== '"')) break;
    if (c === "(") pd++;
    else if (c === ")") { pd--; if (pd === 0) break; }
  }
  if (k >= src.length || src[k] !== ")") {
    const titled = /^(\S+)\s+"[^"\n]*"\)/.exec(src.slice(j + 2));
    if (!titled) return null;
    return { text, url: titled[1], end: j + 2 + titled[0].length };
  }
  return { text, url: src.slice(j + 2, k), end: k + 1 };
}

const PAIRS: [string, (key: string, inner: ReactNode[]) => ReactNode][] = [
  ["**", (key, inner) => <strong key={key}>{inner}</strong>],
  ["__", (key, inner) => <u key={key}>{inner}</u>],
  ["~~", (key, inner) => <s key={key}>{inner}</s>],
  ["||", (key, inner) => <Spoiler key={key}>{inner}</Spoiler>],
];

function inline(src: string, depth: number, noLinks: boolean, key: string): ReactNode[] {
  if (depth > MAX_DEPTH) return [src];
  const out: ReactNode[] = [];
  let buf = "";
  let n = 0;
  const flush = () => { if (buf) { out.push(buf); buf = ""; } };
  const next = () => `${key}.${n++}`;
  const lastIdx: Record<string, number> = {};
  const lastOf = (d: string) => (lastIdx[d] ??= src.lastIndexOf(d));

  let i = 0;
  const len = src.length;
  scan: while (i < len) {
    const c = src[i];

    if (c === "\\" && i + 1 < len && ESCAPABLE.includes(src[i + 1])) {
      buf += src[i + 1];
      i += 2;
      continue;
    }

    if (c === "`") {
      let run = i;
      while (src[run] === "`") run++;
      const ticks = src.slice(i, run);
      let close = src.indexOf(ticks, run);
      while (close !== -1 && (src[close + ticks.length] === "`" || src[close - 1] === "`")) {
        let r = close;
        while (src[r] === "`") r++;
        close = src.indexOf(ticks, r);
      }
      if (close !== -1) {
        let body = src.slice(run, close);
        if (body.length > 2 && body[0] === " " && body[body.length - 1] === " " && body.trim()) body = body.slice(1, -1);
        flush();
        out.push(<code key={next()} className="cm-code">{body}</code>);
        i = close + ticks.length;
        continue;
      }
      buf += ticks;
      i = run;
      continue;
    }

    if (!noLinks && (c === "[" || (c === "!" && src[i + 1] === "["))) {
      const image = c === "!";
      const link = parseLinkAt(src, image ? i + 1 : i);
      if (link) {
        const href = safeUrl(link.url);
        if (href) {
          flush();
          const k = next();
          out.push(<Link key={k} href={href} image={image}>{image ? link.text || "image" : inline(link.text || href, depth + 1, true, k)}</Link>);
        } else {
          buf += link.text;
        }
        i = link.end;
        continue;
      }
    }

    if (!noLinks && (c === "h" || c === "w" || c === "H" || c === "W") && !isWordChar(src[i - 1])) {
      const m = /^(?:https?:\/\/|www\.)[^\s<>"'`]+/i.exec(src.slice(i, i + 2048));
      if (m) {
        let url = m[0];
        while (/[.,:;!?*_~)\]]$/.test(url)) {
          if (url.endsWith(")") && (url.match(/\)/g) ?? []).length <= (url.match(/\(/g) ?? []).length) break;
          url = url.slice(0, -1);
        }
        const href = safeUrl(url);
        if (href && url.length > 4) {
          flush();
          out.push(<Link key={next()} href={href} auto>{url.replace(/^https?:\/\//i, "")}</Link>);
          i += url.length;
          continue;
        }
      }
    }

    for (const [d, build] of PAIRS) {
      if (!src.startsWith(d, i) || lastOf(d) < i + 3) continue;
      if (isSpace(src[i + 2]) || (src[i + 2] === d[0] && d !== "**")) continue;
      const close = findPairClose(src, d, i + 2);
      if (close === -1) continue;
      flush();
      const k = next();
      out.push(build(k, inline(src.slice(i + 2, close), depth + 1, noLinks, k)));
      i = close + 2;
      continue scan;
    }

    if ((c === "*" || c === "_") && src[i + 1] !== c && !isSpace(src[i + 1]) && lastOf(c) > i + 1) {
      if (c === "*" || !isWordChar(src[i - 1])) {
        const close = findSingleClose(src, c, i + 1);
        if (close !== -1 && !(isWordChar(src[i - 1]) && isWordChar(src[close + 1]))) {
          flush();
          const k = next();
          out.push(<em key={k}>{inline(src.slice(i + 1, close), depth + 1, noLinks, k)}</em>);
          i = close + 1;
          continue;
        }
      }
    }

    buf += c;
    i++;
  }
  flush();
  return out;
}

function splitRow(line: string) {
  let s = line.trim();
  if (s.startsWith("|")) s = s.slice(1);
  if (s.endsWith("|") && !s.endsWith("\\|")) s = s.slice(0, -1);
  const cells: string[] = [];
  let cur = "";
  for (let i = 0; i < s.length; i++) {
    if (s[i] === "\\" && s[i + 1] === "|") { cur += "|"; i++; continue; }
    if (s[i] === "|") { cells.push(cur.trim()); cur = ""; continue; }
    cur += s[i];
  }
  cells.push(cur.trim());
  return cells;
}

const isBlockStart = (line: string) => RE_FENCE.test(line) || RE_HEADING.test(line) || RE_QUOTE.test(line) || RE_HR.test(line) || RE_LIST.test(line);

function parseList(lines: string[], start: number, depth: number, key: string) {
  const first = RE_LIST.exec(lines[start])!;
  const baseIndent = indentOf(first[1]);
  const ordered = /\d/.test(first[2]);
  const items: ReactNode[] = [];
  let i = start;
  let tasks = false;
  let loose = false;
  while (i < lines.length) {
    const m = RE_LIST.exec(lines[i]);
    if (!m || indentOf(m[1]) !== baseIndent || /\d/.test(m[2]) !== ordered) break;
    if (RE_HR.test(lines[i]) && !ordered) break;
    const contentIndent = m[1].length + m[2].length + 1;
    const itemLines = [m[3]];
    i++;
    while (i < lines.length) {
      const line = lines[i];
      if (!line.trim()) {
        let k = i;
        while (k < lines.length && !lines[k].trim()) k++;
        if (k < lines.length && indentOf(lines[k]) > baseIndent) {
          for (; i < k; i++) itemLines.push("");
          continue;
        }
        break;
      }
      const ind = indentOf(line);
      if (ind > baseIndent) { itemLines.push(dedent(line, Math.min(ind, contentIndent))); i++; continue; }
      break;
    }
    const task = /^\[([ xX])\][ \t]+/.exec(itemLines[0]);
    const done = !!task && task[1] !== " ";
    if (task) {
      tasks = true;
      itemLines[0] = itemLines[0].slice(task[0].length);
    }
    const k = `${key}.${items.length}`;
    const simple = itemLines.length === 1 || !itemLines.slice(1).some((l) => isBlockStart(l) || !l.trim());
    items.push(
      <li key={k} className={`${task ? "cm-task" : ""} ${done ? "done" : ""}`}>
        {task && <span className={`cm-check ${done ? "done" : ""}`} role="img" aria-label={done ? "done" : "not done"} />}
        <div className="cm-li-body">{simple ? inline(itemLines.join("\n"), depth + 1, false, k) : blocks(itemLines, depth + 1, k)}</div>
      </li>,
    );
    while (i < lines.length && !lines[i].trim()) {
      let j = i;
      while (j < lines.length && !lines[j].trim()) j++;
      const nm = j < lines.length ? RE_LIST.exec(lines[j]) : null;
      if (j - i === 1 && nm && indentOf(nm[1]) === baseIndent && /\d/.test(nm[2]) === ordered) { i = j; loose = true; }
      break;
    }
  }
  const className = `cm-list ${tasks ? "tasks" : ""} ${loose ? "loose" : ""}`;
  const firstNumber = ordered ? parseInt(first[2], 10) : 1;
  const node = ordered ? <ol key={key} className={className} start={firstNumber === 1 ? undefined : firstNumber}>{items}</ol> : <ul key={key} className={className}>{items}</ul>;
  return { node, next: i };
}

function blocks(lines: string[], depth: number, key: string): ReactNode[] {
  if (depth > MAX_DEPTH) return [<p key={key} className="cm-p">{lines.join("\n")}</p>];
  const out: ReactNode[] = [];
  let i = 0;
  let blank = 0;
  const add = (node: (k: string) => ReactNode) => {
    if (out.length && blank > 1) out.push(<div key={`${key}g${out.length}`} className="cm-gap" aria-hidden style={{ height: `${Math.min(blank - 1, MAX_GAP) * 1.5}em` }} />);
    blank = 0;
    out.push(node(`${key}b${out.length}`));
  };
  const startsTable = (at: number) => lines[at].includes("|") && at + 1 < lines.length && RE_TABLE_SEP.test(lines[at + 1]) && lines[at + 1].includes("-");

  while (i < lines.length) {
    const line = lines[i];
    if (!line.trim()) { blank++; i++; continue; }

    const fence = RE_FENCE.exec(line);
    if (fence) {
      const marker = fence[1];
      const body: string[] = [];
      i++;
      while (i < lines.length) {
        const t = lines[i].trim();
        if (t.startsWith(marker[0].repeat(marker.length)) && /^([`~])\1*$/.test(t)) { i++; break; }
        body.push(lines[i]);
        i++;
      }
      add((k) => <CodeBlock key={k} lang={fence[2]} body={body.join("\n")} />);
      continue;
    }

    if (RE_HR.test(line)) { add((k) => <hr key={k} className="cm-hr" />); i++; continue; }

    const h = RE_HEADING.exec(line);
    if (h) {
      add((k) => <div key={k} className={`cm-h cm-h${h[1].length}`} role="heading" aria-level={h[1].length + 2}>{inline(h[2], depth + 1, false, k)}</div>);
      i++;
      continue;
    }

    if (RE_QUOTE.test(line)) {
      const inner: string[] = [];
      while (i < lines.length && RE_QUOTE.test(lines[i])) {
        inner.push(lines[i].replace(RE_QUOTE, ""));
        i++;
      }
      add((k) => <blockquote key={k} className="cm-quote">{blocks(inner, depth + 1, k)}</blockquote>);
      continue;
    }

    if (RE_LIST.test(line)) {
      const at = i;
      let parsed: ReturnType<typeof parseList> | null = null;
      add((k) => { parsed = parseList(lines, at, depth, k); return parsed.node; });
      i = parsed!.next;
      continue;
    }

    if (startsTable(i)) {
      const header = splitRow(line);
      const aligns = splitRow(lines[i + 1]).map((s) => (s.startsWith(":") && s.endsWith(":") ? "center" : s.endsWith(":") ? "right" : undefined));
      if (aligns.length === header.length) {
        i += 2;
        const rows: string[][] = [];
        while (i < lines.length && lines[i].trim() && lines[i].includes("|")) rows.push(splitRow(lines[i++]));
        add((k) => (
          <div key={k} className="cm-table">
            <table>
              <thead><tr>{header.map((cell, c) => <th key={c} style={{ textAlign: aligns[c] as "center" | "right" | undefined }}>{inline(cell, depth + 1, false, `${k}h${c}`)}</th>)}</tr></thead>
              <tbody>{rows.map((r, y) => <tr key={y}>{header.map((_, c) => <td key={c} style={{ textAlign: aligns[c] as "center" | "right" | undefined }}>{inline(r[c] ?? "", depth + 1, false, `${k}r${y}c${c}`)}</td>)}</tr>)}</tbody>
            </table>
          </div>
        ));
        continue;
      }
    }

    const para = [line];
    i++;
    while (i < lines.length && lines[i].trim() && !isBlockStart(lines[i])) {
      if (startsTable(i)) break;
      para.push(lines[i]);
      i++;
    }
    add((k) => <p key={k} className="cm-p">{inline(para.join("\n"), depth + 1, false, k)}</p>);
  }
  return out;
}

function isJumbo(text: string) {
  const t = text.trim();
  if (!t || t.length > 40) return false;
  if (!/^[\p{Extended_Pictographic}\p{Emoji_Component}‍️⃣\s]+$/u.test(t)) return false;
  if (/^[\d#*\s]+$/.test(t)) return false;
  const count = Array.from(new Intl.Segmenter(undefined, { granularity: "grapheme" }).segment(t.replace(/\s+/g, ""))).length;
  return count > 0 && count <= 3;
}

const normalize = (text: string) => text.replace(/\r\n?/g, "\n").replace(/[‪-‮⁦-⁩]/g, "").replace(/^(?:[ \t]*\n)+/, "").replace(/\s+$/, "");

export const ChatMarkdown = memo(function ChatMarkdown({ text }: { text: string }) {
  const src = normalize(text);
  if (isJumbo(src)) return <div className="cm cm-jumbo">{src.trim()}</div>;
  return <div className="cm">{blocks(src.split("\n"), 0, "m")}</div>;
});
