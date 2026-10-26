import { FormEvent, KeyboardEvent, ReactNode, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { AdminStatus, api, AssistantAction, AssistantConversation, AssistantPart, AssistantProgress, AssistantTurn, bytes, Job, Library, Project, Settings } from "../api";
import { Icon } from "./Icon";
import { Action, Progress } from "./Action";
import { Markdown } from "./Markdown";
import { isActive } from "../state";
import { uninstall } from "./projectMenu";
import { fail } from "./Overlays";
import assistantIcon from "../assets/assistant.png";
import assistantGlyph from "../assets/assistant-glyph.png";

type Message = AssistantTurn;
type LivePart = AssistantPart & { startedAt?: number };
type ActionState = { action: AssistantAction; phase: "starting" | "waiting" | "opened" | "failed"; error?: string; previousJob?: Job };
type Run = (id: string, action: AssistantAction, artifactUrl?: string | null) => void;

const PROVIDER: Record<string, string> = { ollama: "Ollama", openai: "OpenAI", anthropic: "Anthropic", gemini: "Gemini", "openai-compatible": "Custom endpoint" };

const projectPayload = (lib: Library) => lib.projects.map((p) => ({ id: p.id, title: p.title, description: p.description.slice(0, 1500), category: p.category, url: p.url, recommended: p.recommended, links: p.links, video: p.video, artifacts: p.artifacts, version: p.version, released: p.released, releaseNotes: p.releaseNotes?.slice(0, 1000) ?? null, installed: !!p.installed?.present, installedVersion: p.installed?.version ?? null, update: p.installed?.update ?? null, installable: !!p.recommended, removable: !!p.installed?.present && !p.installed.external }));

const canBuild = (p: Project, os: string) =>
  p.links.some((l) => /^https?:\/\/github\.com\/[^/]+\/[^/]+/.test(l.url)) && (p.artifacts.length === 0 || p.artifacts.some((a) => a.os === os)) && os !== "android" && os !== "ios";
async function copyText(text: string) {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    const area = document.createElement("textarea");
    area.value = text;
    area.setAttribute("readonly", "");
    area.style.position = "fixed";
    area.style.opacity = "0";
    document.body.appendChild(area);
    area.select();
    const ok = document.execCommand("copy");
    area.remove();
    if (!ok) throw new Error("copy failed");
  }
}
function titleFrom(text: string) {
  let title = "";
  for (const word of text.split(/\s+/).filter(Boolean)) {
    if (title && title.length + 1 + word.length > 40) { title += "…"; break; }
    title += (title ? " " : "") + word;
  }
  if (title.length > 44) title = `${title.slice(0, 40).trimEnd()}…`;
  return title.charAt(0).toUpperCase() + title.slice(1);
}
const firstLine = (e: unknown) => String(e).split("\n")[0].replace(/^Error:\s*/, "");
const stamp = (t: number) => (t > 1e15 ? t / 1e6 : t);

function duration(ms: number) {
  if (ms < 60_000) return `${Math.max(1, Math.round(ms / 1000))}s`;
  const s = Math.round(ms / 1000);
  return `${Math.floor(s / 60)}m ${String(s % 60).padStart(2, "0")}s`;
}

function shortAgo(t: number) {
  const ms = stamp(t);
  const s = Math.max(0, (Date.now() - ms) / 1000);
  if (s < 60) return "now";
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h`;
  const d = new Date(ms);
  if (s < 6 * 86400) return d.toLocaleDateString(undefined, { weekday: "short" });
  return d.toLocaleDateString(undefined, { day: "numeric", month: "short" });
}

function dayGroup(t: number) {
  const start = new Date();
  start.setHours(0, 0, 0, 0);
  const ms = stamp(t);
  if (ms >= start.getTime()) return "Today";
  if (ms >= start.getTime() - 86400_000) return "Yesterday";
  if (ms >= start.getTime() - 7 * 86400_000) return "Previous 7 days";
  return "Earlier";
}

function partsOf(m: Message): AssistantPart[] {
  if (m.parts?.length) return m.parts;
  const parts: AssistantPart[] = [];
  if (m.activity?.length) parts.push({ kind: "thinking", text: m.activity.join("\n\n") });
  for (const s of m.steps ?? []) parts.push({ kind: "tool", text: "", name: s.name, input: s.input, output: s.output, ok: s.ok });
  if (m.content) parts.push({ kind: "text", text: m.content });
  return parts;
}

function lastIndex<T>(list: T[], test: (item: T) => boolean): number {
  for (let i = list.length - 1; i >= 0; i--) if (test(list[i])) return i;
  return -1;
}

function closeThinking(parts: LivePart[], now: number, ms?: number): LivePart[] {
  const at = lastIndex(parts, (p) => p.kind === "thinking" && p.ms === undefined);
  if (at < 0) return parts;
  const next = [...parts];
  next[at] = { ...next[at], ms: ms ?? Math.round(now - (next[at].startedAt ?? now)) };
  return next;
}

function applyEvent(parts: LivePart[], e: AssistantProgress, now: number): LivePart[] {
  const last = parts[parts.length - 1];
  switch (e.kind) {
    case "turn_started":
      return [];
    case "thinking":
      if (last?.kind === "thinking" && last.ms === undefined) return [...parts.slice(0, -1), { ...last, text: last.text + (e.text ?? "") }];
      return [...parts, { kind: "thinking", text: e.text ?? "", startedAt: now }];
    case "thinking_done":
      return closeThinking(parts, now, e.ms);
    case "text": {
      if (last?.kind === "text") return [...parts.slice(0, -1), { ...last, text: last.text + (e.text ?? "") }];
      const text = (e.text ?? "").trimStart();
      return text ? [...closeThinking(parts, now), { kind: "text", text }] : parts;
    }
    case "tool_started":
      return e.part ? [...closeThinking(parts, now), e.part] : parts;
    case "tool_finished": {
      const at = lastIndex(parts, (p) => p.kind === "tool" && p.id === e.part?.id);
      if (at < 0 || !e.part) return parts;
      const next = [...parts];
      next[at] = e.part;
      return next;
    }
    case "error":
      return e.part ? [...closeThinking(parts, now), e.part] : parts;
  }
  return parts;
}

type Step = { t: "thinking"; text: string; ms?: number; startedAt?: number; open: boolean } | { t: "tool"; part: LivePart };
type Group = { t: "process"; steps: Step[] } | { t: "text"; text: string } | { t: "error"; text: string };

function groups(parts: LivePart[]): Group[] {
  const out: Group[] = [];
  for (const p of parts) {
    const last = out[out.length - 1];
    if (p.kind === "thinking" || p.kind === "tool") {
      const run = last?.t === "process" ? last : (out.push({ t: "process", steps: [] }), out[out.length - 1] as Group & { t: "process" });
      const prev = run.steps[run.steps.length - 1];
      if (p.kind === "tool") run.steps.push({ t: "tool", part: p });
      else if (prev?.t === "thinking") {
        prev.text = [prev.text, p.text].filter(Boolean).join("\n\n");
        prev.ms = (prev.ms ?? 0) + (p.ms ?? 0);
        prev.open = p.ms === undefined;
        if (prev.open) prev.startedAt = p.startedAt;
      } else run.steps.push({ t: "thinking", text: p.text, ms: p.ms, startedAt: p.startedAt, open: p.ms === undefined });
    } else if (p.kind === "text") out.push({ t: "text", text: p.text });
    else out.push({ t: "error", text: p.text });
  }
  return out;
}

function suggestionsFor(lib: Library): { icon: string; text: string }[] {
  const installed = lib.projects.filter((p) => p.installed?.present);
  const updates = installed.filter((p) => p.installed?.update);
  const recent = [...installed].sort((a, b) => (b.installed?.installedAt ?? 0) - (a.installed?.installedAt ?? 0))[0];
  const newest = [...lib.projects].filter((p) => p.released && !p.retired).sort((a, b) => (b.released ?? "").localeCompare(a.released ?? ""))[0];
  const out: { icon: string; text: string }[] = [];
  out.push(updates.length ? { icon: "up", text: `Update ${updates.length === 1 ? updates[0].title : `${updates.length} tools`}` } : { icon: "refresh", text: "Anything to update?" });
  if (recent) out.push({ icon: "play", text: `Open ${recent.title}` });
  if (newest) out.push({ icon: "info", text: `What's new in ${newest.title}?` });
  out.push({ icon: "shield", text: "Encrypt a folder before I upload it" });
  return out.slice(0, 4);
}

export function Assistant({ lib, jobs, close, openProject, openSettings, admin, requestAdmin, settings, save, titleBar }: { lib: Library; jobs: Record<string, Job>; close: () => void; openProject: (id: string) => void; openSettings: () => void; admin: AdminStatus | null; requestAdmin: () => Promise<boolean>; settings: Settings; save: (s: Settings) => void; titleBar: boolean }) {
  const [input, setInput] = useState("");
  const [messages, setMessages] = useState<Message[]>([]);
  const [conversations, setConversations] = useState<AssistantConversation[]>([]);
  const [chatId, setChatId] = useState("");
  const [busyChat, setBusyChat] = useState<string | null>(null);
  const [stopping, setStopping] = useState(false);
  const [failed, setFailed] = useState(false);
  const [loading, setLoading] = useState(true);
  const [ready, setReady] = useState(false);
  const [live, setLive] = useState<LivePart[]>([]);
  const [away, setAway] = useState(false);
  const [history, setHistory] = useState(false);
  const [deleted, setDeleted] = useState<AssistantConversation | null>(null);
  const [actionStates, setActionStates] = useState<Record<string, ActionState>>({});
  const projects = useMemo(() => new Map(lib.projects.map((p) => [p.id, p])), [lib.projects]);
  const suggestions = useMemo(() => suggestionsFor(lib), [lib]);
  const chatRef = useRef("");
  chatRef.current = chatId;
  const busyRef = useRef<string | null>(null);
  const scroller = useRef<HTMLDivElement>(null);
  const atBottom = useRef(true);
  const field = useRef<HTMLTextAreaElement>(null);
  const pendingDelete = useRef<{ id: string; timer: number } | null>(null);
  const model = settings.assistantModel || "no model";
  const provider = PROVIDER[settings.assistantProvider] ?? settings.assistantProvider;
  const busyHere = busyChat !== null && busyChat === chatId;
  const busyElsewhere = busyChat !== null && busyChat !== chatId;

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let alive = true;
    let queue: AssistantProgress[] = [];
    let frame = 0;
    const flush = () => {
      frame = 0;
      const batch = queue;
      queue = [];
      const now = performance.now();
      setLive((parts) => batch.reduce((p, e) => applyEvent(p, e, now), parts));
    };
    void api.onAssistantProgress((event) => {
      if (event.chatId !== busyRef.current) return;
      queue.push(event);
      if (!frame) frame = requestAnimationFrame(flush);
    }).then((stop) => { if (alive) unlisten = stop; else stop(); });
    return () => { alive = false; unlisten?.(); if (frame) cancelAnimationFrame(frame); };
  }, []);

  useEffect(() => { const t = window.setTimeout(() => setReady(true), 150); return () => window.clearTimeout(t); }, []);
  useEffect(() => { if (!loading) setReady(true); }, [loading]);

  useLayoutEffect(() => {
    const el = scroller.current;
    if (el && atBottom.current) el.scrollTop = el.scrollHeight;
  }, [messages, live, busyChat, chatId]);

  useLayoutEffect(() => {
    const el = field.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 180)}px`;
  }, [input]);

  const commitDelete = () => {
    const pending = pendingDelete.current;
    if (!pending) return;
    window.clearTimeout(pending.timer);
    pendingDelete.current = null;
    setDeleted(null);
    api.assistantDeleteConversation(pending.id).catch(fail);
  };
  useEffect(() => () => commitDelete(), []);

  const loadConversation = async (id: string) => {
    const thread = await api.assistantGetConversation(id);
    setChatId(id);
    setMessages(thread.messages);
    atBottom.current = true;
    setAway(false);
  };

  useEffect(() => {
    let alive = true;
    api.assistantConversations().then(async (items) => {
      if (!alive) return;
      setConversations(items);
      const latest = items.find((c) => c.messages.length) ?? items[0];
      if (latest) await loadConversation(latest.id);
      else {
        const created = await api.assistantNewConversation();
        if (alive) { setConversations([created]); setChatId(created.id); setMessages([]); }
      }
    }).catch((e) => { if (alive) setMessages([{ role: "assistant", content: "", parts: [{ kind: "error", text: `Couldn’t load your chats: ${firstLine(e)}. They’re still on disk.` }] }]); })
      .finally(() => { if (alive) setLoading(false); });
    return () => { alive = false; };
  }, []);

  useEffect(() => { if (!loading) field.current?.focus(); }, [loading, chatId]);

  const say = (text: string) => setMessages((all) => [...all, { role: "assistant", content: "", parts: [{ kind: "error", text }] }]);

  const unlockIfNeeded = async () => {
    if (lib.host.os !== "linux") return true;
    let access = admin;
    try { access ??= await api.adminStatus(); }
    catch {
      say("Couldn’t check administrator access, so I didn’t start anything. Try again in a moment.");
      return false;
    }
    if (access.supported && !access.unlocked) {
      const unlocked = await requestAdmin();
      if (!unlocked) say("Nothing changed. Administrator access is still locked. Unlock it and ask again.");
      return unlocked;
    }
    return true;
  };

  const runAction = async (id: string, action: AssistantAction, artifactUrl?: string | null, alreadyUnlocked = false) => {
    const existing = jobs[id];
    const expectedOperation = action === "uninstall" ? "uninstall" : action === "download" ? "download" : action === "build" ? "build" : action === "launch" ? null : "install";
    if (action !== "launch" && isActive(existing)) {
      if (existing.operation !== expectedOperation) setActionStates((all) => ({ ...all, [id]: { action, phase: "failed", error: `A ${existing.operation} is already running. Try again when it’s done.`, previousJob: existing } }));
      return;
    }
    if (!alreadyUnlocked && ["install", "update_now", "build"].includes(action) && !(await unlockIfNeeded())) return;
    setActionStates((all) => ({ ...all, [id]: { action, phase: "starting", previousJob: existing } }));
    try {
      if (action === "download") await api.download(id, artifactUrl ?? undefined);
      else if (action === "build") await api.build(id);
      else if (action === "launch") await api.launch(id);
      else await api.install(id);
      setActionStates((all) => ({ ...all, [id]: { action, phase: action === "launch" ? "opened" : "waiting", previousJob: existing } }));
    } catch (e) {
      setActionStates((all) => ({ ...all, [id]: { action, phase: "failed", error: firstLine(e), previousJob: existing } }));
    }
  };
  const run: Run = (id, action, url) => void runAction(id, action, url);

  const createConversation = async () => {
    try {
      const created = await api.assistantNewConversation();
      setConversations((all) => [created, ...all.filter((c) => c.id !== created.id)]);
      setChatId(created.id);
      setMessages([]);
      setInput("");
      setHistory(false);
      atBottom.current = true;
      field.current?.focus();
    } catch (e) { fail(e); }
  };

  const selectConversation = (id: string) => {
    setHistory(false);
    if (id === chatId) return;
    loadConversation(id).catch(fail);
  };

  const removeConversation = (id: string) => {
    const conv = conversations.find((c) => c.id === id);
    if (!conv || busyChat === id) return;
    commitDelete();
    setConversations((all) => all.filter((c) => c.id !== id));
    setDeleted(conv);
    pendingDelete.current = { id, timer: window.setTimeout(commitDelete, 6000) };
    if (id === chatId) void createConversation();
  };

  const undoDelete = () => {
    const pending = pendingDelete.current;
    if (!pending || !deleted) return;
    window.clearTimeout(pending.timer);
    pendingDelete.current = null;
    const back = deleted;
    setConversations((all) => [...all, back].sort((a, b) => b.updatedAt - a.updatedAt));
    setDeleted(null);
  };

  const send = (text: string) => {
    const query = text.trim();
    const target = chatId;
    if (!query || !target || busyChat || loading) return;
    setInput("");
    setMessages((all) => [...all, { role: "user", content: query }]);
    setConversations((all) => all.map((c) => c.id === target ? { ...c, title: c.messages.length ? c.title : titleFrom(query), messages: [...c.messages, { role: "user", content: query }], updatedAt: Date.now() * 1e6 } : c));
    busyRef.current = target;
    setBusyChat(target);
    setStopping(false);
    setLive([]);
    atBottom.current = true;
    setAway(false);
    const done = () => { busyRef.current = null; setBusyChat(null); setLive([]); };
    api.assistantChat(target, query, projectPayload(lib))
      .then(async (reply) => {
        const answer: Message = { role: "assistant", content: reply.text, projectIds: reply.projectIds, artifactUrls: reply.artifactUrls, action: reply.action, parts: reply.parts, model: reply.model, elapsedMs: reply.elapsedMs, stopped: reply.stopped };
        if (chatRef.current === target) setMessages((all) => [...all, answer]);
        setFailed(!reply.text && reply.parts.some((p) => p.kind === "error"));
        done();
        if (["install", "download", "update_now", "launch", "build"].includes(reply.action) && reply.projectIds.length) {
          const needsUnlock = ["install", "update_now", "build"].includes(reply.action);
          if (needsUnlock && !(await unlockIfNeeded())) return;
          for (const [index, id] of reply.projectIds.entries()) await runAction(id, reply.action, reply.artifactUrls?.[index], needsUnlock);
        }
        const items = await api.assistantConversations();
        setConversations(items.filter((c) => c.id !== pendingDelete.current?.id));
      })
      .catch((e) => { setFailed(true); if (chatRef.current === target) say(`That didn’t work: ${firstLine(e)}`); })
      .finally(done);
  };

  const lastQuestion = [...messages].reverse().find((m) => m.role === "user")?.content;
  const retry = lastQuestion && !busyChat ? () => send(lastQuestion) : undefined;

  const stop = () => {
    if (!busyChat || stopping) return;
    setStopping(true);
    api.assistantStop(busyChat).catch(fail);
  };

  const submit = (e: FormEvent) => { e.preventDefault(); send(input); };
  const onKey = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) { e.preventDefault(); send(input); }
  };
  const onDialogKey = (e: KeyboardEvent<HTMLElement>) => {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "n" && !busyHere) { e.preventDefault(); void createConversation(); }
  };

  const asked = useRef<"no" | "chat" | "sent">("no");
  useEffect(() => {
    const q = import.meta.env.DEV ? new URLSearchParams(location.hash.slice(1)).get("ask") : null;
    if (!q || loading || !chatId || asked.current === "sent") return;
    if (asked.current === "no") { asked.current = "chat"; void createConversation(); }
    else if (!messages.length) { asked.current = "sent"; send(q); }
  });

  const doAction = (p: Project, msg: Message) => {
    if (msg.action === "uninstall") void uninstall(p);
    else if (msg.action && ["install", "download", "update_now", "build"].includes(msg.action)) void runAction(p.id, msg.action, msg.artifactUrls?.[msg.projectIds?.indexOf(p.id) ?? 0]);
    else if (msg.action === "launch") void api.launch(p.id).catch(fail);
    else openProject(p.id);
  };

  const current = conversations.find((c) => c.id === chatId);
  const empty = !loading && messages.length === 0 && !busyHere;
  const visible = conversations.filter((c) => c.messages.length > 0);
  const updates = lib.projects.filter((p) => p.installed?.present && p.installed.update).length;
  const installedCount = lib.projects.filter((p) => p.installed?.present).length;

  const list = (
    <ChatList chats={visible} chatId={chatId} empty={empty} busyChat={busyChat} loading={loading && ready} mac={lib.host.os === "macos"} newChat={() => void createConversation()} select={selectConversation} remove={removeConversation} />
  );

  return (
    <div className={`assistant-scrim fixed inset-x-0 bottom-0 z-[80] grid place-items-center min-[1280px]:p-6 ${titleBar ? "top-11" : "top-0"}`} onMouseDown={(e) => e.target === e.currentTarget && close()}>
      <section role="dialog" aria-modal="true" aria-labelledby="assistant-title" data-ready={ready || undefined} onKeyDown={onDialogKey}
        className="assistant relative flex h-full w-full overflow-hidden bg-card min-[1280px]:h-[min(860px,100%)] min-[1280px]:max-w-[1160px] min-[1280px]:rounded-[var(--r-lg)] min-[1280px]:border min-[1280px]:border-line-2 min-[1280px]:shadow-[var(--shadow)]">
        <aside className="flex w-[248px] shrink-0 flex-col border-r border-line bg-side/60 max-[880px]:hidden">
          <div className="flex h-[52px] shrink-0 items-center gap-2.5 border-b border-line px-[18px]">
            <img src={assistantGlyph} alt="" draggable={false} className="h-6 w-6 shrink-0" />
            <span className="text-[14.5px] font-semibold">Assistant</span>
          </div>
          {list}
        </aside>

        <div className="flex min-w-0 flex-1 flex-col">
          <header className="flex h-[52px] shrink-0 items-center gap-1.5 border-b border-line pr-3 pl-5 max-[880px]:pl-3">
            <div className="relative min-[881px]:hidden">
              <button className="assistant-ghost" aria-label="Chat history" title="Chat history" aria-expanded={history} onClick={() => setHistory((v) => !v)}><Icon name="list" /></button>
              {history && (
                <>
                  <div className="fixed inset-0 z-10" onMouseDown={() => setHistory(false)} />
                  <div className="ctx absolute top-9 left-0 z-20 flex max-h-[60vh] w-[280px] flex-col overflow-hidden rounded-[var(--r-md)] border border-line-2 bg-card shadow-[var(--shadow)]">{list}</div>
                </>
              )}
            </div>
            <button className="assistant-ghost min-[881px]:hidden" aria-label="New chat" title="New chat (Ctrl+N)" onClick={() => void createConversation()}><Icon name="plus" /></button>
            <h2 id="assistant-title" className={`min-w-0 flex-1 truncate ${empty ? "text-[13px] text-ink-3" : "text-[14px] font-semibold"}`}>{empty || !current ? "New chat" : current.title}</h2>
            <button className="assistant-model" title={`${provider}. Click to change the model.`} onClick={openSettings}>
              {(busyChat || failed) && <i className={busyChat ? "busy" : "bad"} />}
              <span className="max-w-[220px] truncate">{model}</span>
            </button>
            <button className="assistant-ghost" aria-label="Close assistant" title="Close (Esc)" onClick={close}><Icon name="x" /></button>
          </header>

          <div className="relative min-h-0 flex-1">
            <div ref={scroller} className="scroll h-full px-6" aria-live="polite" onScroll={(e) => {
              const el = e.currentTarget;
              const near = el.scrollHeight - el.scrollTop - el.clientHeight < 140;
              atBottom.current = near;
              if (near === away) setAway(!near);
            }}>
              {empty ? (
                <div className="mx-auto flex h-full w-full max-w-[720px] flex-col justify-end pb-5">
                  <img src={assistantIcon} alt="" draggable={false} className="mb-4 h-11 w-11" />
                  <h3 className="font-head text-[22px] font-semibold tracking-[-.02em]">Describe the job. I’ll find the tool.</h3>
                  <p className="mt-1.5 text-[13px] text-ink-2">
                    {updates > 0 && <><span className="text-[var(--mark-ink)]">{updates} {updates === 1 ? "update" : "updates"} waiting</span> · </>}
                    {installedCount} installed · {lib.projects.length} in the catalogue. Nothing changes without your OK.
                  </p>
                  <div className="mt-4 flex flex-col">
                    {suggestions.map((s) => (
                      <button key={s.text} className="assistant-suggestion group/s" disabled={!chatId || !!busyChat} onClick={() => send(s.text)}>
                        <Icon name={s.icon} /><span className="min-w-0 flex-1 truncate">{s.text}</span><kbd className="kbd opacity-0 transition-opacity group-hover/s:opacity-100">↵</kbd>
                      </button>
                    ))}
                  </div>
                </div>
              ) : (
                <div className="mx-auto flex w-full max-w-[720px] flex-col gap-8 pt-6 pb-8">
                  {messages.map((m, i) => m.role === "user"
                    ? <UserMessage key={`${chatId}-${i}`} text={m.content} />
                    : <AssistantMessage key={`${chatId}-${i}`} m={m} model={model} retry={i === messages.length - 1 ? retry : undefined} lib={lib} jobs={jobs} projects={projects} actionStates={actionStates} doAction={doAction} openProject={openProject} run={run} />)}
                  {busyHere && <LiveMessage parts={live} thinking={settings.assistantThinking} model={model} stopping={stopping} projects={projects} />}
                </div>
              )}
            </div>
            {away && busyHere && (
              <button className="assistant-jump" aria-label="Jump to the latest" title="Jump to the latest" onClick={() => { const el = scroller.current; if (el) el.scrollTo({ top: el.scrollHeight, behavior: "smooth" }); }}><Icon name="down" /></button>
            )}
          </div>

          <form onSubmit={submit} className="shrink-0 px-6 pt-1 pb-5">
            <div className="assistant-composer mx-auto max-w-[720px]">
              <textarea ref={field} rows={1} className="assistant-input" value={input} onChange={(e) => setInput(e.target.value)} onKeyDown={onKey}
                placeholder={busyElsewhere ? "Another chat is still answering…" : loading ? "" : "Describe a task or name a tool"} disabled={loading || !chatId} aria-label="Message" />
              <div className="flex items-center gap-2 pr-2 pb-2 pl-4">
                <button type="button" className={`assistant-toggle -ml-2 ${settings.assistantThinking ? "on" : ""}`} aria-pressed={settings.assistantThinking} title="Reason before answering. Slower, often better. Models that can’t think ignore it." onClick={() => save({ ...settings, assistantThinking: !settings.assistantThinking })}>Think first</button>
                <span className="ml-auto flex items-center gap-1 text-[11px] text-ink-3">
                  {busyHere ? (stopping ? "Stopping…" : <><kbd className="kbd">Esc</kbd> to hide · the reply keeps going</>) : input && !input.includes("\n") ? <><kbd className="kbd">⇧</kbd><kbd className="kbd">↵</kbd> new line</> : null}
                </span>
                {busyHere
                  ? <button type="button" className="assistant-send stop" aria-label="Stop" title="Stop" disabled={stopping} onClick={stop}><Icon name="stop" /></button>
                  : <button className="assistant-send" aria-label="Send" title="Send" disabled={!input.trim() || loading || !chatId || !!busyChat}><Icon name="up" /></button>}
              </div>
            </div>
          </form>
        </div>

        {deleted && (
          <div className="toast absolute bottom-4 left-4 z-30 flex items-center gap-3 rounded-[var(--r-md)] py-2 pr-2 pl-3.5 text-[13px]" role="status">
            <span className="max-w-[220px] truncate">Deleted “{deleted.title}”</span>
            <button className="assistant-link" onClick={undoDelete}>Undo</button>
          </div>
        )}
      </section>
    </div>
  );
}

function ChatList({ chats, chatId, empty, busyChat, loading, mac, newChat, select, remove }: { chats: AssistantConversation[]; chatId: string; empty: boolean; busyChat: string | null; loading: boolean; mac: boolean; newChat: () => void; select: (id: string) => void; remove: (id: string) => void }) {
  const labelled = chats.length > 8;
  let lastGroup = "";
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="px-2 pt-2">
        <button className={`assistant-row w-full ${empty ? "on" : ""}`} onClick={newChat}>
          <Icon name="plus" /><span className="flex-1 text-left">New chat</span><kbd className="kbd">{mac ? "⌘ N" : "Ctrl N"}</kbd>
        </button>
      </div>
      <nav className="scroll min-h-0 flex-1 px-2 pt-1 pb-3">
        {loading && chats.length === 0 && [0, 1, 2].map((i) => <div key={i} className="skeleton mx-2.5 my-2 h-4 rounded-[var(--r-sm)]" style={{ width: `${70 - i * 12}%` }} />)}
        {chats.map((c) => {
          const group = dayGroup(c.updatedAt);
          const header = labelled && group !== lastGroup ? group : null;
          lastGroup = group;
          const answering = busyChat === c.id;
          return (
            <div key={c.id}>
              {header && <div className="px-2.5 pt-3 pb-1 text-[11px] font-medium text-ink-3">{header}</div>}
              <div className={`assistant-chat group/chat ${c.id === chatId && !empty ? "on" : ""}`}>
                <button className="flex h-8 min-w-0 flex-1 items-center gap-2 pl-2.5 text-left" onClick={() => select(c.id)} title={c.title}>
                  {answering && <i className="assistant-dot" aria-hidden />}
                  <span className="min-w-0 flex-1 truncate text-[13px]">{c.title}</span>
                  <span className={`pr-2 font-mono text-[11px] text-ink-3 ${answering ? "" : "group-hover/chat:hidden group-focus-within/chat:hidden"}`}>{answering ? "Answering" : shortAgo(c.updatedAt)}</span>
                </button>
                {!answering && <button className="assistant-chat-del hidden group-hover/chat:grid group-focus-within/chat:grid" aria-label={`Delete ${c.title}`} title="Delete chat" onClick={() => remove(c.id)}><Icon name="trash" /></button>}
              </div>
            </div>
          );
        })}
      </nav>
    </div>
  );
}

function UserMessage({ text }: { text: string }) {
  return (
    <div className="flex justify-end">
      <div className="assistant-user selectable max-w-[85%] whitespace-pre-wrap break-words">{text}</div>
    </div>
  );
}

function LiveMessage({ parts, thinking, model, stopping, projects }: { parts: LivePart[]; thinking: boolean; model: string; stopping: boolean; projects: Map<string, Project> }) {
  const list = groups(parts);
  const last = parts[parts.length - 1];
  const waiting = !last || (last.kind === "tool" && last.ok !== undefined) || (last.kind === "thinking" && last.ms !== undefined);
  return (
    <div className="flex flex-col gap-3.5">
      {list.map((g, i) => {
        if (g.t === "process") return <div key={i} className="flex flex-col gap-0.5">{g.steps.map((s, j) => s.t === "thinking" ? <ThinkingStep key={j} step={s} /> : <ToolStep key={s.part.id ?? j} p={s.part} projects={projects} />)}</div>;
        if (g.t === "error") return <ErrorNote key={i} text={g.text} />;
        return <Markdown key={i} text={g.text} streaming={i === list.length - 1} className="selectable" />;
      })}
      {waiting && (
        <div className="assistant-step">
          <span className="assistant-breathe">{stopping ? "Stopping" : !last ? (thinking ? "Thinking" : `Waiting for ${model}`) : "Working"}</span>
        </div>
      )}
    </div>
  );
}

function AssistantMessage({ m, model, retry, lib, jobs, projects, actionStates, doAction, openProject, run }: { m: Message; model: string; retry?: () => void; lib: Library; jobs: Record<string, Job>; projects: Map<string, Project>; actionStates: Record<string, ActionState>; doAction: (p: Project, m: Message) => void; openProject: (id: string) => void; run: Run }) {
  const [copied, setCopied] = useState(false);
  const list = groups(partsOf(m));
  const cards = (m.projectIds ?? []).map((id) => projects.get(id)).filter((p): p is Project => !!p);
  const copy = () => {
    copyText(m.content).then(() => { setCopied(true); window.setTimeout(() => setCopied(false), 1400); }).catch(() => {});
  };
  const errorOnly = !m.content && list.every((g) => g.t !== "text");
  return (
    <div className="group/msg flex flex-col gap-3.5">
      {list.map((g, i) => {
        if (g.t === "process") return <ProcessSummary key={i} steps={g.steps} projects={projects} />;
        if (g.t === "error") return <ErrorNote key={i} text={g.text} retry={errorOnly ? retry : undefined} />;
        return <Markdown key={i} text={g.text} className="selectable" />;
      })}
      {cards.length > 0 && (
        <div className="grid gap-2">
          {cards.map((p) => m.action && m.action !== "none"
            ? <AssistantActionCard key={p.id} project={p} action={m.action} job={jobs[p.id]} actionState={actionStates[p.id]} retry={() => doAction(p, m)} open={() => openProject(p.id)} />
            : <ProjectCard key={p.id} p={p} lib={lib} job={jobs[p.id]} open={() => openProject(p.id)} run={run} />)}
        </div>
      )}
      {(m.content || m.stopped) && (
        <div className="-mt-1.5 -ml-1.5 flex h-6 items-center gap-2 font-mono text-[11px] text-ink-3 opacity-0 transition-opacity group-hover/msg:opacity-100 focus-within:opacity-100">
          {m.content && <button className="assistant-ghost !h-6 !w-6" onClick={copy} title={copied ? "Copied" : "Copy"} aria-label="Copy answer"><Icon name={copied ? "check" : "copy"} className={copied ? "text-ok" : ""} /></button>}
          {m.stopped && <span className="inline-flex items-center gap-1"><Icon name="stop" className="!h-3 !w-3" />stopped</span>}
          {m.elapsedMs ? <span>{duration(m.elapsedMs)}</span> : null}
          {m.model && m.model !== model && <span>{m.model}</span>}
        </div>
      )}
    </div>
  );
}

function ErrorNote({ text, retry }: { text: string; retry?: () => void }) {
  return (
    <div className="assistant-error">
      <Icon name="warn" />
      <span className="selectable min-w-0 flex-1">{text}</span>
      {retry && <button className="assistant-link shrink-0" onClick={retry}>Try again</button>}
    </div>
  );
}

function ProcessSummary({ steps, projects }: { steps: Step[]; projects: Map<string, Project> }) {
  const [open, setOpen] = useState(() => import.meta.env.DEV && new URLSearchParams(location.hash.slice(1)).has("expand"));
  const tools = steps.filter((s): s is Step & { t: "tool" } => s.t === "tool");
  const ms = steps.reduce((sum, s) => sum + (s.t === "thinking" ? s.ms ?? 0 : 0), 0);
  const failedCount = tools.filter((s) => s.part.ok === false).length;
  const label = tools.length === 0
    ? ms ? `Thought for ${duration(ms)}` : "Thought briefly"
    : `${ms ? `Worked for ${duration(ms)}` : "Worked"} · ${tools.length} ${tools.length === 1 ? "step" : "steps"}`;
  return (
    <div>
      <button type="button" className="assistant-step group/step" aria-expanded={open} onClick={() => setOpen((v) => !v)}>
        <span>{label}</span>
        {failedCount > 0 && <span className="text-bad">· {failedCount} failed</span>}
        <Icon name="down" className={`assistant-chev ${open ? "rotate-180 !opacity-100" : ""}`} />
      </button>
      {open && (
        <div className="assistant-rule flex flex-col gap-0.5">
          {steps.map((s, i) => s.t === "thinking"
            ? <div key={i} className="py-1"><Markdown text={s.text || "(The model didn’t share its thinking.)"} className="assistant-thought" /></div>
            : <ToolStep key={s.part.id ?? i} p={s.part} projects={projects} />)}
        </div>
      )}
    </div>
  );
}

function ticker(text: string) {
  const flat = text.replace(/\*\*|__|`/g, "").replace(/\s+/g, " ").trim();
  const ends = [". ", "! ", "? "].map((e) => flat.lastIndexOf(e, flat.length - 3));
  let cut = Math.max(...ends);
  if (cut > 0 && flat.length - cut < 30) cut = Math.max(...[". ", "! ", "? "].map((e) => flat.lastIndexOf(e, cut - 1)));
  const sentence = cut >= 0 ? flat.slice(cut + 2) : flat;
  return sentence.length > 140 ? `…${sentence.slice(-140)}` : sentence;
}

function ThinkingStep({ step }: { step: Step & { t: "thinking" } }) {
  const [open, setOpen] = useState(false);
  const [, tick] = useState(0);
  useEffect(() => {
    if (!step.open) return;
    const t = window.setInterval(() => tick((n) => n + 1), 1000);
    return () => window.clearInterval(t);
  }, [step.open]);
  const elapsed = step.open ? (step.startedAt ? performance.now() - step.startedAt : 0) + (step.ms ?? 0) : step.ms;
  return (
    <div>
      <button type="button" className="assistant-step group/step" aria-expanded={open} disabled={!step.text} onClick={() => setOpen((v) => !v)}>
        {step.open
          ? <><span className="assistant-breathe">Thinking</span>{elapsed ? <span className="font-mono text-[11px]">{duration(elapsed)}</span> : null}</>
          : <span>{elapsed ? `Thought for ${duration(elapsed)}` : "Thought briefly"}</span>}
        {step.text && <Icon name="down" className={`assistant-chev ${open ? "rotate-180 !opacity-100" : ""}`} />}
      </button>
      {step.open && !open && step.text && <div className="assistant-ticker">{ticker(step.text)}</div>}
      {open && step.text && <div className="assistant-rule"><Markdown text={step.text} className="assistant-thought selectable" /></div>}
    </div>
  );
}

const TOOL: Record<string, { icon: string; doing: string; done: string }> = {
  search_projects: { icon: "search", doing: "Searching the catalogue for", done: "Searched the catalogue for" },
  search_tools: { icon: "search", doing: "Searching the catalogue for", done: "Searched the catalogue for" },
  get_project_details: { icon: "info", doing: "Reading about", done: "Read about" },
  get_tool_details: { icon: "info", doing: "Reading about", done: "Read about" },
  get_project_links: { icon: "link", doing: "Looking up links for", done: "Looked up links for" },
  get_project_video: { icon: "play", doing: "Looking up the video for", done: "Looked up the video for" },
  list_installed: { icon: "box", doing: "Checking what’s installed", done: "Checked what’s installed" },
  check_updates: { icon: "refresh", doing: "Checking for updates", done: "Checked for updates" },
  get_system_info: { icon: "monitor", doing: "Checking this computer", done: "Checked this computer" },
  about_zsync: { icon: "user", doing: "Reading about ZSync and TheHolyOneZ", done: "Read about ZSync and TheHolyOneZ" },
  get_project_readme: { icon: "book", doing: "Reading the README of", done: "Read the README of" },
  search_project_text: { icon: "regex", doing: "Searching", done: "Searched" },
  get_github_info: { icon: "branch", doing: "Checking GitHub for", done: "Checked GitHub for" },
  install_project: { icon: "download", doing: "Preparing to install", done: "Prepared to install" },
  download_project: { icon: "download", doing: "Preparing the download of", done: "Prepared the download of" },
  update_project: { icon: "up", doing: "Preparing to update", done: "Prepared to update" },
  update_all: { icon: "up", doing: "Preparing all updates", done: "Prepared all updates" },
  uninstall_project: { icon: "trash", doing: "Preparing removal of", done: "Prepared removal of" },
  launch_project: { icon: "play", doing: "Getting ready to open", done: "Ready to open" },
  build_project: { icon: "code", doing: "Setting up a source build of", done: "Set up a source build of" },
};

function toolText(p: LivePart, projects: Map<string, Project>) {
  const fallback = (p.name ?? "tool").replace(/_/g, " ");
  const meta = TOOL[p.name ?? ""] ?? { icon: "gear", doing: `Running ${fallback}`, done: `Ran ${fallback}` };
  const running = p.ok === undefined;
  const input = p.input ?? {};
  const id = typeof input.project_id === "string" ? input.project_id : typeof input.id === "string" ? input.id : null;
  const subject = typeof input.query === "string" ? `“${input.query}”` : id ? projects.get(id)?.title ?? id : "";
  const detail = typeof input.pattern === "string" ? `/${input.pattern}/` : typeof input.section === "string" && input.section ? `› ${input.section}` : "";
  return { icon: meta.icon, verb: running ? meta.doing : meta.done, subject, detail, running };
}

function ToolStep({ p, projects }: { p: LivePart; projects: Map<string, Project> }) {
  const [open, setOpen] = useState(false);
  const { icon, verb, subject, detail, running } = toolText(p, projects);
  const bad = p.ok === false;
  const plainSubject = subject.replace(/[“”]/g, "");
  const section = typeof p.input?.section === "string" ? p.input.section : "";
  const repeats = (t: string) => t === plainSubject || t.startsWith(`${plainSubject} `) || (!!section && t.includes(section));
  const summary = !running && p.summary && !repeats(p.summary) ? p.summary : null;
  const args = Object.entries(p.input ?? {});
  return (
    <div>
      <button type="button" className="assistant-step group/step" aria-expanded={open} onClick={() => setOpen((v) => !v)} disabled={running}>
        {running ? <span className="assistant-spinner" aria-label="Running" /> : <Icon name={bad ? "warn" : icon} className={bad ? "text-bad" : ""} />}
        <span className="min-w-0 truncate">
          <span className={bad ? "text-bad" : ""}>{verb}</span>
          {subject && <span className="font-medium text-ink-2"> {subject}</span>}
          {detail && <span className="ml-1.5 font-mono text-[12px]">{detail}</span>}
          {summary && <span className={bad ? "text-bad" : ""}> · {summary}</span>}
        </span>
        {!running && <Icon name="down" className={`assistant-chev ${open ? "rotate-180 !opacity-100" : ""}`} />}
      </button>
      {open && (
        <div className="assistant-rule grid gap-2 py-1.5">
          {args.length > 0 && (
            <div className="flex flex-wrap gap-1.5">
              {args.map(([k, v]) => <span key={k} className="assistant-arg"><span className="text-ink-3">{k.replace(/_/g, " ")}</span> {typeof v === "string" ? v : JSON.stringify(v)}</span>)}
            </div>
          )}
          {p.output && <pre className="assistant-output selectable">{p.output}</pre>}
        </div>
      )}
    </div>
  );
}

function Poster({ p }: { p: Project }) {
  const [broken, setBroken] = useState(false);
  const poster = p.video?.poster;
  return (
    <span className="assistant-poster">
      {poster && !broken ? <img src={poster} alt="" loading="lazy" onError={() => setBroken(true)} /> : <span>{p.title.replace(/^Z(?=[A-Z])/, "").slice(0, 1)}</span>}
    </span>
  );
}

function cardStatus(p: Project, os: string): { text: string; tone?: "upd" } {
  const inst = p.installed?.present ? p.installed : null;
  if (inst?.update) return { text: `Update ${inst.update}`, tone: "upd" };
  if (inst) return { text: "Installed" };
  if (p.recommended) return { text: "Not installed" };
  if (canBuild(p, os)) return { text: "Source build" };
  return { text: "Not available here" };
}

function CardShell({ open, title, children, side, tone = "", sub }: { open: () => void; title: string; children?: ReactNode; side?: ReactNode; tone?: string; sub?: ReactNode }) {
  return (
    <div className={`assistant-card group/card ${tone}`} role="button" tabIndex={0} onClick={open} onKeyDown={(e) => { if (e.target === e.currentTarget && (e.key === "Enter" || e.key === " ")) { e.preventDefault(); open(); } }} title={`Open ${title}`}>
      {children}
      <span className="min-w-0 flex-1">{sub}</span>
      {side && <span className="flex shrink-0 items-center gap-1.5" onClick={(e) => e.stopPropagation()}>{side}</span>}
      <Icon name="back" className="!h-3.5 !w-3.5 shrink-0 rotate-180 text-ink-3 opacity-0 transition-opacity group-hover/card:opacity-100" />
    </div>
  );
}

function ProjectCard({ p, lib, job, open, run }: { p: Project; lib: Library; job?: Job; open: () => void; run: Run }) {
  const status = cardStatus(p, lib.host.os);
  const buildable = canBuild(p, lib.host.os);
  const buildHere = !p.installed?.present && !p.recommended && buildable && !isActive(job);
  const nothingHere = !p.installed?.present && !p.recommended && !buildable && !p.artifacts.some((a) => a.os === lib.host.os);
  const side = buildHere
    ? <button className="btn sm go" onClick={() => run(p.id, "build")}><Icon name="code" /> Build</button>
    : nothingHere
      ? p.url ? <button className="assistant-ghost" title="Website" aria-label="Website" onClick={() => api.openUrl(p.url!).catch(fail)}><Icon name="link" /></button> : null
      : <Action p={p} lib={lib} job={job} compact />;
  return (
    <CardShell open={open} title={p.title} side={side} sub={
      <>
        <span className="block truncate text-[14.5px] font-semibold">{p.title}</span>
        <span className="block truncate text-[12px] text-ink-3">
          <span className={status.tone ? "text-[var(--mark-ink)]" : ""}>{status.text}</span>
          {[p.version ? `v${p.version}` : null, p.category].filter(Boolean).map((t) => ` · ${t}`).join("")}
        </span>
      </>
    }><Poster p={p} /></CardShell>
  );
}

function AssistantActionCard({ project, action, job, actionState, retry, open }: { project: Project; action: AssistantAction; job?: Job; actionState?: ActionState; retry: () => void; open: () => void }) {
  const expectedOperation = action === "uninstall" ? "uninstall" : action === "download" ? "download" : action === "build" ? "build" : action === "launch" ? null : "install";
  const freshJob = !actionState || job !== actionState.previousJob;
  const visibleJob = job && (isActive(job) || freshJob && job.operation === expectedOperation) ? job : undefined;
  const stop = () => api.cancel(project.id).catch(fail);
  const openApp = () => api.launch(project.id).catch(fail);
  const showFile = (path: string) => api.reveal(path).catch(fail);
  const title = project.title;
  const verb = action === "update_now" ? "Updating" : action === "download" ? "Downloading" : action === "build" ? "Building" : action === "launch" ? "Opening" : action === "uninstall" ? "Removing" : "Installing";
  const base = action === "update_now" ? "update" : action === "download" ? "download" : action === "build" ? "build" : action === "launch" ? "open" : action === "uninstall" ? "uninstall" : "install";
  const rec = project.artifacts.find((a) => a.url === project.recommended);

  const card = (tone: string, head: ReactNode, sub?: ReactNode, side?: ReactNode) => (
    <CardShell open={open} title={title} tone={tone} side={side} sub={<><span className="block truncate text-[14.5px] font-semibold">{head}</span>{sub && <span className="block truncate text-[12px] text-ink-3">{sub}</span>}</>}>
      <Poster p={project} />
    </CardShell>
  );

  if (visibleJob && isActive(visibleJob)) return card("live", `${verb} ${title}`, <span className="mt-1 block"><Progress job={visibleJob} /></span>,
    (visibleJob.phase === "downloading" || visibleJob.phase === "building") && <button className="assistant-ghost" title="Stop" aria-label="Stop" onClick={stop}><Icon name="x" /></button>);

  if (actionState?.phase === "failed" || visibleJob?.phase === "failed") return card("bad", <span className="text-bad">{title} didn’t {base}</span>, actionState?.error ?? visibleJob?.error ?? visibleJob?.hint, <button className="btn sm" onClick={retry}>Try again</button>);

  if (visibleJob?.phase === "cancelled") return card("", `${title} stopped`, "You stopped it.", <button className="btn sm" onClick={retry}>Resume</button>);

  if (visibleJob?.phase === "done") {
    if (visibleJob.step === "Removed") return card("", `${title} removed`, "It’s no longer on this computer.");
    if (visibleJob.savedTo) return card("ok", `${title} downloaded`, visibleJob.savedTo, <button className="btn sm" onClick={() => showFile(visibleJob.savedTo!)}><Icon name="folder" /> Show file</button>);
    if (project.installed?.present) return card("ok", `${title} is ready`, project.installed.version ? `Version ${project.installed.version}` : "Installed on this computer", <button className="btn sm go" onClick={openApp}><Icon name="play" /> Open</button>);
    return card("", `Finishing ${title}…`);
  }

  if (actionState?.phase === "starting" || actionState?.phase === "waiting") return card("live", <span className="flex items-center gap-2"><span className="assistant-spinner" />{verb} {title}…</span>);
  if (actionState?.phase === "opened") return card("ok", `Opened ${title}`, undefined, <button className="btn sm" onClick={openApp}><Icon name="play" /> Open again</button>);
  if (action === "uninstall") return card("", `Remove ${title}?`, "You’ll confirm before anything is deleted.", <button className="btn sm danger" onClick={retry}>Remove…</button>);
  if (action === "launch") return card("", title, "Installed", <button className="btn sm" onClick={openApp}><Icon name="play" /> Open</button>);

  const label = action === "download" ? "Download" : action === "build" ? "Build" : action === "update_now" ? "Update" : "Install";
  const sub = action === "build" ? "Builds from source" : action === "update_now" ? `Update ${project.installed?.update ?? ""}`.trim() : `Not installed${rec?.size ? ` · ${bytes(rec.size)}` : ""}`;
  return card("", title, sub, <button className="btn sm go" onClick={retry}>{label}</button>);
}
