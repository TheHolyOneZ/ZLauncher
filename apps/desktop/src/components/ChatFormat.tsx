import { KeyboardEvent, ClipboardEvent, RefObject, useEffect, useRef, useState } from "react";
import { Icon } from "./Icon";

type Format = { wrap?: string; prefix?: string; fence?: boolean; link?: boolean };

const mac = /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);
const MOD = mac ? "⌘" : "Ctrl";

const TOOLS: ("sep" | [string, string, string, Format])[] = [
  ["bold", "Bold", `${MOD}+B`, { wrap: "**" }],
  ["italic", "Italic", `${MOD}+I`, { wrap: "*" }],
  ["underline", "Underline", `${MOD}+U`, { wrap: "__" }],
  ["strike", "Strikethrough", `${MOD}+Shift+X`, { wrap: "~~" }],
  "sep",
  ["code", "Inline code", `${MOD}+E`, { wrap: "`" }],
  ["codeblock", "Code block", `${MOD}+Shift+E`, { fence: true }],
  ["link", "Link", `${MOD}+K`, { link: true }],
  "sep",
  ["quote", "Quote", `${MOD}+Shift+.`, { prefix: "> " }],
  ["list", "Bulleted list", `${MOD}+Shift+8`, { prefix: "- " }],
  ["olist", "Numbered list", `${MOD}+Shift+7`, { prefix: "n" }],
  ["task", "Checklist", "", { prefix: "- [ ] " }],
  ["eyeoff", "Spoiler", `${MOD}+Shift+S`, { wrap: "||" }],
];

const CHEATSHEET: [string, string][] = [
  ["**bold**", "bold"],
  ["*italic*", "italic"],
  ["__underline__", "underline"],
  ["~~strike~~", "strikethrough"],
  ["||spoiler||", "hidden until clicked"],
  ["`code`", "inline code"],
  ["```js", "code block (close with ```)"],
  ["# Heading", "heading (#, ##, ###)"],
  ["> quote", "quote"],
  ["- item", "bullet list"],
  ["1. item", "numbered list"],
  ["- [ ] task", "checklist"],
  ["[text](https://…)", "link"],
  ["---", "divider"],
  ["\\*", "show a symbol literally"],
];

const valueSetter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set;

function replaceRange(ta: HTMLTextAreaElement, start: number, end: number, text: string, selStart: number, selEnd = selStart) {
  ta.focus();
  ta.setSelectionRange(start, end);
  let ok = false;
  try {
    ok = document.execCommand("insertText", false, text);
  } catch {
    ok = false;
  }
  if (!ok) {
    valueSetter?.call(ta, ta.value.slice(0, start) + text + ta.value.slice(end));
    ta.dispatchEvent(new Event("input", { bubbles: true }));
  }
  ta.setSelectionRange(selStart, selEnd);
}

function toggleWrap(ta: HTMLTextAreaElement, mark: string) {
  const v = ta.value;
  const s = ta.selectionStart;
  const e = ta.selectionEnd;
  const L = mark.length;
  if (s >= L && v.slice(s - L, s) === mark && v.slice(e, e + L) === mark) {
    replaceRange(ta, s - L, e + L, v.slice(s, e), s - L, e - L);
    return;
  }
  const sel = v.slice(s, e);
  if (sel.length > 2 * L && sel.startsWith(mark) && sel.endsWith(mark)) {
    replaceRange(ta, s, e, sel.slice(L, -L), s, e - 2 * L);
    return;
  }
  const m = /^(\s*)([\s\S]*?)(\s*)$/.exec(sel);
  const lead = m?.[1] ?? "";
  const trail = m?.[3] ?? "";
  const core = m ? m[2] : sel;
  const cs = s + lead.length + L;
  replaceRange(ta, s, e, lead + mark + core + mark + trail, cs, cs + core.length);
}

function lineBounds(v: string, s: number, e: number): [number, number] {
  const ls = v.lastIndexOf("\n", s - 1) + 1;
  let le = v.indexOf("\n", e > s && v[e - 1] === "\n" ? e - 1 : e);
  if (le === -1) le = v.length;
  return [ls, le];
}

function togglePrefix(ta: HTMLTextAreaElement, prefix: string) {
  const v = ta.value;
  const [ls, le] = lineBounds(v, ta.selectionStart, ta.selectionEnd);
  const lines = v.slice(ls, le).split("\n");
  const numbered = prefix === "n";
  const re = numbered ? /^(\s*)\d+[.)] / : new RegExp(`^(\\s*)${prefix.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}`);
  const all = lines.filter((l) => l.trim()).every((l) => re.test(l));
  let k = 0;
  const out = lines.map((l) => {
    if (!l.trim() && lines.length > 1) return l;
    if (all) return l.replace(re, "$1");
    const stripped = l.replace(/^(\s*)(?:[-*+] \[[ xX]\] |[-*+] |\d+[.)] |> )/, "$1");
    k++;
    const ind = /^\s*/.exec(stripped)![0];
    return ind + (numbered ? `${k}. ` : prefix) + stripped.slice(ind.length);
  }).join("\n");
  replaceRange(ta, ls, le, out, lines.length === 1 ? ls + out.length : ls, ls + out.length);
}

function insertFence(ta: HTMLTextAreaElement) {
  const v = ta.value;
  const s = ta.selectionStart;
  const e = ta.selectionEnd;
  const sel = v.slice(s, e);
  const before = s > 0 && v[s - 1] !== "\n" ? "\n" : "";
  const after = e < v.length && v[e] !== "\n" ? "\n" : "";
  const out = `${before}\`\`\`\n${sel}${sel && !sel.endsWith("\n") ? "\n" : ""}\`\`\`${after}`;
  const caret = s + before.length + 3;
  replaceRange(ta, s, e, out, caret);
}

function insertLink(ta: HTMLTextAreaElement) {
  const s = ta.selectionStart;
  const e = ta.selectionEnd;
  const sel = ta.value.slice(s, e);
  if (/^(https?:\/\/|www\.)\S+$/i.test(sel.trim())) {
    replaceRange(ta, s, e, `[](${sel.trim()})`, s + 1);
    return;
  }
  const label = sel || "text";
  const out = `[${label}](https://)`;
  const url = s + 1 + label.length + 2;
  if (sel) replaceRange(ta, s, e, out, url, url + 8);
  else replaceRange(ta, s, e, out, s + 1, s + 5);
}

export function applyFormat(ta: HTMLTextAreaElement, f: Format) {
  if (f.wrap) toggleWrap(ta, f.wrap);
  else if (f.prefix) togglePrefix(ta, f.prefix);
  else if (f.fence) insertFence(ta);
  else if (f.link) insertLink(ta);
}

function inOpenFence(v: string, pos: number): boolean | "opening" {
  const before = v.slice(0, pos).split("\n");
  let open = false;
  for (const l of before.slice(0, -1)) if (/^ {0,3}(```|~~~)/.test(l)) open = !open;
  if (/^ {0,3}```[\w+#.-]*\s*$/.test(before[before.length - 1]) && !open) return "opening";
  return open;
}

function continueList(ta: HTMLTextAreaElement) {
  const v = ta.value;
  const s = ta.selectionStart;
  if (s !== ta.selectionEnd) return false;
  const ls = v.lastIndexOf("\n", s - 1) + 1;
  const line = v.slice(ls, s);
  const m = /^(\s*)((?:[-*+] \[[ xX]\] )|(?:[-*+•] )|(?:(\d+)([.)]) )|(?:> ))(.*)$/.exec(line);
  if (!m) return false;
  if (!m[5].trim() && s === ls + line.length) {
    replaceRange(ta, ls, s, m[1], ls + m[1].length);
    return true;
  }
  let marker = m[2];
  if (m[3]) marker = `${parseInt(m[3], 10) + 1}${m[4]} `;
  else if (/\[[xX]\]/.test(marker)) marker = marker.replace(/\[[xX]\]/, "[ ]");
  const ins = `\n${m[1]}${marker}`;
  replaceRange(ta, s, s, ins, s + ins.length);
  return true;
}

function indentLines(ta: HTMLTextAreaElement, outdent: boolean) {
  const v = ta.value;
  const [ls, le] = lineBounds(v, ta.selectionStart, ta.selectionEnd);
  const lines = v.slice(ls, le).split("\n");
  const out = lines.map((l) => (outdent ? l.replace(/^( {1,2}|\t)/, "") : `  ${l}`)).join("\n");
  const single = lines.length === 1 && ta.selectionStart === ta.selectionEnd;
  const delta = out.length - (le - ls);
  const caret = Math.max(ls, ta.selectionStart + (outdent ? Math.max(delta, -(ta.selectionStart - ls)) : 2));
  replaceRange(ta, ls, le, out, single ? caret : ls, single ? caret : ls + out.length);
}

export function formatKey(ev: KeyboardEvent<HTMLTextAreaElement>, send?: () => void, togglePreview?: () => void) {
  const ta = ev.currentTarget;
  const mod = mac ? ev.metaKey : ev.ctrlKey;
  const key = ev.key.length === 1 ? ev.key.toLowerCase() : ev.key;
  const stop = () => { ev.preventDefault(); ev.stopPropagation(); };

  if (mod && !ev.altKey) {
    let act: Format | null = null;
    if (!ev.shiftKey && key === "b") act = { wrap: "**" };
    else if (!ev.shiftKey && key === "i") act = { wrap: "*" };
    else if (!ev.shiftKey && key === "u") act = { wrap: "__" };
    else if (!ev.shiftKey && key === "e") act = { wrap: "`" };
    else if (!ev.shiftKey && key === "k") act = { link: true };
    else if (ev.shiftKey && key === "x") act = { wrap: "~~" };
    else if (ev.shiftKey && key === "e") act = { fence: true };
    else if (ev.shiftKey && key === "s") act = { wrap: "||" };
    else if (ev.shiftKey && (key === ">" || key === "." || ev.code === "Period")) act = { prefix: "> " };
    else if (ev.shiftKey && (key === "*" || ev.code === "Digit8")) act = { prefix: "- " };
    else if (ev.shiftKey && (key === "&" || ev.code === "Digit7")) act = { prefix: "n" };
    if (act) { stop(); applyFormat(ta, act); return; }
    if (ev.shiftKey && key === "p" && togglePreview) { stop(); togglePreview(); return; }
    if (key === "Enter" && send) { stop(); send(); return; }
  }

  if (ev.key === "Tab" && !ev.altKey && !mod) {
    const v = ta.value;
    const [ls] = lineBounds(v, ta.selectionStart, ta.selectionEnd);
    const end = v.indexOf("\n", ls);
    const line = v.slice(ls, end === -1 ? v.length : end);
    const fenced = inOpenFence(v, ta.selectionStart) === true;
    if (fenced || /^\s*([-*+]|\d+[.)]) /.test(line)) {
      ev.preventDefault();
      if (fenced && !ev.shiftKey && ta.selectionStart === ta.selectionEnd) replaceRange(ta, ta.selectionStart, ta.selectionEnd, "  ", ta.selectionStart + 2);
      else indentLines(ta, ev.shiftKey);
      return;
    }
  }

  if (ev.key === "Enter" && !ev.nativeEvent.isComposing) {
    const fence = inOpenFence(ta.value, ta.selectionStart);
    if (ev.shiftKey || !send || fence) {
      if (continueList(ta)) { ev.preventDefault(); return; }
      if (fence === "opening" && !ev.shiftKey && send) {
        ev.preventDefault();
        const s = ta.selectionStart;
        const closed = /\n```\s*(\n|$)/.test(ta.value.slice(s));
        replaceRange(ta, s, ta.selectionEnd, closed ? "\n" : "\n\n```", s + 1);
      }
      return;
    }
    ev.preventDefault();
    send();
  }
}

export function formatPaste(ev: ClipboardEvent<HTMLTextAreaElement>) {
  const ta = ev.currentTarget;
  const text = ev.clipboardData.getData("text/plain").trim();
  const s = ta.selectionStart;
  const e = ta.selectionEnd;
  const sel = ta.value.slice(s, e);
  if (sel && !/\n/.test(sel) && /^(https?:\/\/)\S+$/i.test(text) && !/^(https?:\/\/)/i.test(sel)) {
    ev.preventDefault();
    const out = `[${sel}](${text})`;
    replaceRange(ta, s, e, out, s + out.length);
  }
}

export function FormatBar({ field, preview, setPreview }: { field: RefObject<HTMLTextAreaElement | null>; preview: boolean; setPreview: (on: boolean) => void }) {
  const [help, setHelp] = useState(false);
  const box = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!help) return;
    const away = (e: MouseEvent) => { if (!box.current?.contains(e.target as Node)) setHelp(false); };
    document.addEventListener("mousedown", away);
    return () => document.removeEventListener("mousedown", away);
  }, [help]);
  return (
    <div ref={box} className="ttz-format" role="toolbar" aria-label="Formatting">
      {TOOLS.map((t, i) => t === "sep" ? <span key={i} className="ttz-format-sep" /> : (
        <button key={t[0]} type="button" title={t[2] ? `${t[1]} (${t[2]})` : t[1]} aria-label={t[1]} disabled={preview}
          onMouseDown={(e) => e.preventDefault()} onClick={() => field.current && applyFormat(field.current, t[3])}><Icon name={t[0]} /></button>
      ))}
      <span className="ml-auto" />
      <button type="button" title="Formatting help" aria-label="Formatting help" aria-expanded={help} className={help ? "on" : ""} onMouseDown={(e) => e.preventDefault()} onClick={() => setHelp(!help)}><Icon name="help" /></button>
      <button type="button" title={`Preview (${MOD}+Shift+P)`} aria-label="Preview" aria-pressed={preview} className={preview ? "on" : ""} onMouseDown={(e) => e.preventDefault()} onClick={() => setPreview(!preview)}><Icon name="eye" /></button>
      {help && (
        <div className="ttz-format-help scroll">
          <div className="grid grid-cols-[auto_1fr] items-baseline gap-x-3 gap-y-0.5">
            {CHEATSHEET.map(([syntax, what]) => [<code key={syntax} className="font-mono text-[12px] text-ink">{syntax}</code>, <span key={what} className="text-[12px] text-ink-3">{what}</span>])}
          </div>
          <p className="mt-2.5 border-t border-line pt-2 text-[11.5px] text-ink-3">Enter sends · Shift+Enter new line · {MOD}+Enter always sends.</p>
        </div>
      )}
    </div>
  );
}
