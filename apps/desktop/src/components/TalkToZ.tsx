import React, { FormEvent, KeyboardEvent, ReactNode, useCallback, useEffect, useRef, useState } from "react";
import { api, TtzChat, TtzFile, TtzFileKind, TtzKind, TtzMessage, TtzPreview, TtzStaged, TtzState } from "../api";
import ttzIcon from "../assets/ttz.png";
import profile from "../assets/theholyonez.jpg";
import { ChatMarkdown } from "./ChatMarkdown";
import { FormatBar, formatKey, formatPaste } from "./ChatFormat";
import { Segmented } from "./Controls";
import { Icon } from "./Icon";
import { ask, fail } from "./Overlays";

const KINDS: { value: TtzKind; label: string; icon: string }[] = [
  { value: "HELP", label: "Help", icon: "info" },
  { value: "REQUEST", label: "Request", icon: "plus" },
  { value: "FEEDBACK", label: "Feedback", icon: "chat" },
  { value: "COLLAB", label: "Collab", icon: "branch" },
  { value: "GENERAL", label: "General", icon: "globe" },
];
const STATUS: Record<string, string> = { OPEN: "Open", IN_PROGRESS: "TheHolyOneZ is on it", CLOSED: "Closed", ARCHIVED: "Archived" };
const POLL_MS = 6000;
const MAX_FILES = 5;
const FILE_ICONS: Record<TtzFileKind, string> = { image: "image", video: "film", archive: "box", document: "file", text: "code" };
const FILE_LABELS: Record<TtzFileKind, string> = { image: "Picture", video: "Video", archive: "Zip", document: "PDF", text: "Text" };

function fileSize(n: number) {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${Math.round(n / 1024)} KB`;
  return `${(n / 1024 / 1024).toFixed(n < 10 * 1024 * 1024 ? 1 : 0)} MB`;
}

let previews: Promise<unknown> = Promise.resolve();
function preview(chat: string, file: string): Promise<TtzPreview> {
  const run = () => api.ttzFilePreview(chat, file);
  const next = previews.then(run, run);
  previews = next.catch(() => {});
  return next;
}

function when(s: string) {
  const d = new Date(s.replace(" ", "T") + "Z");
  return isNaN(+d) ? new Date() : d;
}
const clock = (d: Date) => d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
function dayLabel(d: Date) {
  const today = new Date();
  const yesterday = new Date(today);
  yesterday.setDate(today.getDate() - 1);
  if (d.toDateString() === today.toDateString()) return "Today";
  if (d.toDateString() === yesterday.toDateString()) return "Yesterday";
  return d.toLocaleDateString([], { weekday: "long", month: "long", day: "numeric", ...(d.getFullYear() !== today.getFullYear() ? { year: "numeric" } : {}) });
}
function shortAgo(d: Date) {
  const s = (Date.now() - +d) / 1000;
  if (s < 60) return "now";
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h`;
  if (s < 6 * 86400) return d.toLocaleDateString([], { weekday: "short" });
  return d.toLocaleDateString([], { day: "numeric", month: "short" });
}
const isClosed = (c: TtzChat) => c.status === "CLOSED" || c.status === "ARCHIVED";
const titleOf = (c: TtzChat) => c.subject ?? (c.readable ? "Chat" : "Chat from your browser");

export function TalkToZ({ close, titleBar, mac, start }: { close: () => void; titleBar: boolean; mac: boolean; start?: string | null }) {
  const [st, setSt] = useState<TtzState | null>(null);
  const [sel, setSel] = useState<string | null>(null);
  const [thread, setThread] = useState<{ id: string; messages: TtzMessage[]; rev: number } | null>(null);
  const [threadError, setThreadError] = useState<string | null>(null);
  const [syncing, setSyncing] = useState(false);
  const [signing, setSigning] = useState(false);
  const [draft, setDraft] = useState("");
  const [sending, setSending] = useState(false);
  const [staged, setStaged] = useState<(TtzStaged & { done?: number })[]>([]);
  const [previewing, setPreviewing] = useState(false);
  const scroller = useRef<HTMLDivElement>(null);
  const field = useRef<HTMLTextAreaElement>(null);
  const chats = useRef<TtzChat[]>([]);
  chats.current = st?.chats ?? [];
  const selected = useRef<string | null>(null);
  selected.current = sel;
  const chat = st?.chats.find((c) => c.id === sel) ?? null;

  const refresh = useCallback(async (server: boolean) => {
    if (server) setSyncing(true);
    try {
      const s = await api.ttzState(server);
      setSt(s);
      return s;
    } catch (e) {
      fail(e);
    } finally {
      if (server) setSyncing(false);
    }
  }, []);

  const patch = useCallback((c: TtzChat) => {
    setSt((s) => s && { ...s, chats: s.chats.some((x) => x.id === c.id) ? s.chats.map((x) => (x.id === c.id ? c : x)) : [c, ...s.chats] });
  }, []);

  useEffect(() => {
    let first = true;
    void refresh(false).then((s) => {
      if (s && first) setSel(start ?? s.chats[0]?.id ?? null);
      first = false;
      void refresh(true);
    });
    const off = api.onTtz(() => void refresh(false));
    return () => void off.then((f) => f());
  }, [refresh, start]);

  const attach = useCallback((list: TtzStaged[]) => {
    const problems = list.filter((f) => f.error).map((f) => `${f.name}: ${f.error}`);
    if (problems.length) fail(problems.join("\n"));
    const good = list.filter((f) => !f.error);
    setStaged((cur) => {
      const room = MAX_FILES - cur.length;
      if (good.length > room) fail(`A message can carry up to ${MAX_FILES} files.`);
      good.slice(Math.max(room, 0)).forEach((f) => void api.ttzUnstage(f.id));
      return [...cur, ...good.slice(0, Math.max(room, 0))];
    });
  }, []);

  useEffect(() => {
    const dropped = api.onTtzDropped((list) => {
      const c = chats.current.find((x) => x.id === selected.current);
      if (c?.readable && !isClosed(c)) attach(list);
      else list.forEach((f) => void api.ttzUnstage(f.id));
    });
    const uploading = api.onTtzUpload((p) => setStaged((cur) => cur.map((f) => (f.id === p.id ? { ...f, done: p.done } : f))));
    return () => {
      void dropped.then((f) => f());
      void uploading.then((f) => f());
    };
  }, [attach]);

  useEffect(() => {
    setThread(null);
    setThreadError(null);
    setStaged((cur) => {
      cur.forEach((f) => void api.ttzUnstage(f.id));
      return [];
    });
    const c = chats.current.find((x) => x.id === sel);
    if (!sel || (c && !c.readable)) return;
    let live = true;
    api.ttzMessages(sel, 0, -1).then(
      (t) => {
        if (!live) return;
        setThread({ id: sel, messages: t.messages, rev: t.rev });
        patch(t.chat);
      },
      (e) => live && setThreadError(String(e)),
    );
    return () => {
      live = false;
    };
  }, [sel, patch]);

  const lastId = thread?.messages.at(-1)?.id ?? 0;
  const threadId = thread?.id;
  const rev = thread?.rev ?? -1;
  const open = !!chat && !isClosed(chat);
  useEffect(() => {
    if (!threadId || !open) return;
    const t = window.setInterval(() => {
      if (document.hidden) return;
      api.ttzMessages(threadId, lastId, rev).then(
        (r) => {
          setThread((cur) => {
            if (!cur || cur.id !== threadId) return cur;
            const changed = new Map(r.changed.map((m) => [m.id, m]));
            const known = cur.messages.map((m) => changed.get(m.id) ?? m);
            return { ...cur, rev: r.rev, messages: [...known, ...r.messages.filter((m) => !known.some((x) => x.id === m.id))] };
          });
          patch(r.chat);
        },
        () => {},
      );
    }, POLL_MS);
    return () => window.clearInterval(t);
  }, [threadId, lastId, rev, open, patch]);

  const replaced = useCallback((m: TtzMessage) => {
    setThread((cur) => cur && { ...cur, messages: cur.messages.map((x) => (x.id === m.id ? m : x)) });
  }, []);

  useEffect(() => {
    const el = scroller.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [thread?.id, thread?.messages.length]);

  async function send(e?: FormEvent) {
    e?.preventDefault();
    const text = draft.trim();
    if ((!text && !staged.length) || !sel || sending) return;
    setSending(true);
    try {
      const m = await api.ttzSend(sel, text, staged.map((f) => f.id));
      setThread((t) => (t && t.id === sel ? { ...t, messages: [...t.messages, m] } : t));
      setDraft("");
      setStaged([]);
      setPreviewing(false);
      field.current?.focus();
    } catch (e) {
      setStaged((cur) => cur.map((f) => ({ ...f, done: undefined })));
      fail(e);
    } finally {
      setSending(false);
    }
  }

  function unstage(id: string) {
    void api.ttzUnstage(id);
    setStaged((cur) => cur.filter((f) => f.id !== id));
  }

  async function signIn() {
    setSigning(true);
    try {
      await api.ttzSignIn();
      await refresh(true);
    } catch (e) {
      if (!String(e).toLowerCase().includes("cancelled")) fail(e);
    } finally {
      setSigning(false);
    }
  }

  async function signOut() {
    const ok = await ask({ title: "Sign out of TalkToZ?", body: "This launcher stops showing your account's chats until you sign in again. Anonymous chats stay.", ok: "Sign out" });
    if (!ok) return;
    try {
      await api.ttzSignOut();
      const s = await refresh(false);
      if (s && sel && !s.chats.some((c) => c.id === sel)) setSel(s.chats[0]?.id ?? null);
    } catch (e) {
      fail(e);
    }
  }

  async function forget(c: TtzChat) {
    const ok = await ask({
      title: `Remove “${titleOf(c)}”?`,
      body: "Only this launcher has the key to this chat, so you won't be able to read it again. TheHolyOneZ keeps his copy.",
      ok: "Remove",
      danger: true,
    });
    if (!ok) return;
    try {
      await api.ttzForget(c.id);
      const s = await refresh(false);
      if (sel === c.id) setSel(s?.chats[0]?.id ?? null);
    } catch (e) {
      fail(e);
    }
  }

  function onDialogKey(e: KeyboardEvent) {
    if (e.key === "Escape" && !e.defaultPrevented) {
      e.preventDefault();
      close();
    }
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "n") {
      e.preventDefault();
      setSel(null);
    }
  }

  const unreadable = !!chat && !chat.readable;

  return (
    <div className={`assistant-scrim fixed inset-x-0 bottom-0 z-[80] grid place-items-center min-[1280px]:p-6 ${titleBar ? "top-11" : "top-0"}`} onMouseDown={(e) => e.target === e.currentTarget && close()}>
      <section role="dialog" aria-modal="true" aria-labelledby="ttz-title" data-ready onKeyDown={onDialogKey}
        className="assistant relative flex h-full w-full overflow-hidden bg-bg min-[1280px]:h-[min(860px,100%)] min-[1280px]:max-w-[1160px] min-[1280px]:rounded-[var(--r-lg)] min-[1280px]:border min-[1280px]:border-line-2 min-[1280px]:shadow-[var(--shadow)]">
        <aside className="flex w-[260px] shrink-0 flex-col border-r border-line max-[880px]:hidden">
          <div className="flex h-[52px] shrink-0 items-center gap-2.5 border-b border-line px-4">
            <img src={ttzIcon} alt="" draggable={false} className="h-6 w-6 shrink-0 rounded-[6px]" />
            <span className="text-[14.5px] font-semibold">Talk to TheZ</span>
            {syncing && <Icon name="refresh" className="spin ml-auto !h-3.5 !w-3.5 text-ink-3" />}
          </div>
          <div className="px-2 pt-2">
            <button className={`assistant-row w-full ${sel === null ? "on" : ""}`} onClick={() => setSel(null)}>
              <Icon name="plus" /><span className="flex-1 text-left">New chat</span><kbd className="kbd">{mac ? "⌘ N" : "Ctrl N"}</kbd>
            </button>
          </div>
          <nav className="scroll min-h-0 flex-1 px-2 pt-1 pb-3">
            {!st && [0, 1, 2].map((i) => <div key={i} className="skeleton mx-2.5 my-2 h-4 rounded-[var(--r-sm)]" style={{ width: `${70 - i * 12}%` }} />)}
            {st?.chats.map((c) => (
              <div key={c.id} className={`assistant-chat group/chat ${c.id === sel ? "on" : ""}`}>
                <button className="flex min-w-0 flex-1 items-center gap-2 py-1.5 pl-2.5 text-left" onClick={() => setSel(c.id)} title={titleOf(c)}>
                  <span className="min-w-0 flex-1">
                    <span className={`block truncate text-[13px] ${c.unread ? "font-semibold text-ink" : ""}`}>{titleOf(c)}</span>
                    <span className="flex items-center gap-1.5 text-[11px] text-ink-3">
                      <i className={`ttz-status ${c.status.toLowerCase()}`} aria-hidden />
                      {STATUS[c.status] ?? c.status}
                      {c.anonymous && <> · anonymous</>}
                    </span>
                  </span>
                  <span className={`pr-2 font-mono text-[11px] ${c.unread ? "text-cyan" : "text-ink-3"} ${c.readable ? "group-hover/chat:hidden group-focus-within/chat:hidden" : ""}`}>
                    {c.unread ? "new" : shortAgo(when(c.lastAt))}
                  </span>
                </button>
                {c.readable && <button className="assistant-chat-del hidden group-hover/chat:grid group-focus-within/chat:grid" aria-label={`Remove ${titleOf(c)}`} title="Remove from this launcher" onClick={() => void forget(c)}><Icon name="trash" /></button>}
              </div>
            ))}
          </nav>
          <Account st={st} signing={signing} signIn={signIn} signOut={signOut} />
        </aside>

        <div className="flex min-w-0 flex-1 flex-col">
          <header className="flex h-[52px] shrink-0 items-center gap-2 border-b border-line pr-3 pl-5 max-[880px]:pl-3">
            <button className="assistant-ghost min-[881px]:hidden" aria-label="New chat" title="New chat" onClick={() => setSel(null)}><Icon name="plus" /></button>
            <h2 id="ttz-title" className="min-w-0 flex-1 truncate text-[14px] font-semibold">{chat ? titleOf(chat) : "New chat"}</h2>
            {chat && <span className="ttz-pill"><i className={`ttz-status ${chat.status.toLowerCase()}`} aria-hidden />{STATUS[chat.status] ?? chat.status}</span>}
            <span className="ttz-pill" title="Only you and TheHolyOneZ can read this chat. The server only stores ciphertext."><Icon name="lock" /> End-to-end</span>
            <button className="assistant-ghost" aria-label="Close" title="Close (Esc)" onClick={close}><Icon name="x" /></button>
          </header>

          {st?.error && (
            <div className="ttz-banner" role="status"><Icon name="warn" /><span className="min-w-0 flex-1">{st.error}</span><button className="assistant-link" onClick={() => void refresh(true)}>Try again</button></div>
          )}

          {sel === null ? (
            <NewChat st={st} signing={signing} signIn={signIn} created={async (c) => { await refresh(false); setSel(c.id); }} />
          ) : unreadable ? (
            <Centered>
              <Icon name="lock" className="!h-6 !w-6 text-ink-3" />
              <h3 className="mt-3 text-[17px] font-semibold">This chat lives in your browser</h3>
              <p className="mt-1 max-w-[46ch] text-[13.5px] text-ink-2">It was started on the website, and the key that unlocks it stayed there. Open it in that browser to read it. New chats you start here can be read here.</p>
              <button className="btn mt-4" onClick={() => api.openUrl(`https://zsync.eu/ttz/chat/${encodeURIComponent(chat.id)}`).catch(fail)}><Icon name="external" /> Open in browser</button>
            </Centered>
          ) : (
            <>
              <div ref={scroller} className="scroll min-h-0 flex-1 px-6" aria-live="polite">
                <div className="mx-auto flex w-full max-w-[720px] flex-col pt-6 pb-6">
                  {threadError ? <p className="ttz-banner mx-0">{threadError}</p>
                    : !thread ? [0, 1, 2].map((i) => <div key={i} className={`skeleton my-2 h-10 rounded-[var(--r-md)] ${i % 2 ? "self-end" : ""}`} style={{ width: `${55 - i * 8}%` }} />)
                      : <Messages chat={thread.id} open={open} messages={thread.messages} anonymousAs={chat?.anonymous ? chat.name : undefined} replaced={replaced} />}
                </div>
              </div>
              {chat && isClosed(chat) ? (
                <div className="ttz-closed"><Icon name="lock" /> TheHolyOneZ closed this chat. Start a new one any time.</div>
              ) : (
                <form onSubmit={send} className="shrink-0 px-6 pt-1 pb-5">
                  <div className="assistant-composer mx-auto max-w-[720px]">
                    <FormatBar field={field} preview={previewing} setPreview={setPreviewing} />
                    {staged.length > 0 && (
                      <ul className="flex flex-wrap gap-1.5 px-3 pt-3" aria-label="Files to send">
                        {staged.map((f) => (
                          <li key={f.id} className="ttz-chip" style={{ "--done": `${Math.round((f.done ?? 0) * 100)}%` } as React.CSSProperties}>
                            <span className="min-w-0 truncate">{f.name}</span>
                            <span className="shrink-0 text-ink-3">{fileSize(f.size)}</span>
                            {f.metadata && <span className="ttz-note shrink-0" title="Location and device data is removed from JPEG and PNG photos. This format can keep it.">may keep location data</span>}
                            <button type="button" aria-label={`Remove ${f.name}`} title="Remove" disabled={sending} onClick={() => unstage(f.id)}><Icon name="x" /></button>
                          </li>
                        ))}
                      </ul>
                    )}
                    {previewing && <div className="ttz-preview">{draft.trim() ? <ChatMarkdown text={draft} /> : <span className="text-ink-3">Nothing to preview yet.</span>}</div>}
                    <textarea ref={field} rows={1} className={`assistant-input ${previewing ? "hidden" : ""}`} value={draft} onChange={(e) => setDraft(e.target.value)} onPaste={formatPaste} aria-label="Message"
                      placeholder={st?.open === false ? "TalkToZ isn't taking messages right now" : "Write to TheHolyOneZ"} disabled={!thread || st?.open === false}
                      onKeyDown={(e) => formatKey(e, () => void send(), () => setPreviewing((p) => !p))} />
                    <div className="flex items-center gap-2 pr-2 pb-2 pl-2">
                      <button type="button" className="assistant-ghost" aria-label="Attach files" title="Attach files, or drop them here. Up to 100 MB each." disabled={!thread || sending || st?.open === false || staged.length >= MAX_FILES}
                        onClick={() => api.ttzPickFiles().then(attach).catch(fail)}><Icon name="clip" /></button>
                      <span className="text-[11px] text-ink-3">{chat?.anonymous ? `Anonymous${chat.name ? ` as ${chat.name}` : ""}` : st?.account ? `As ${st.account.username}` : ""}</span>
                      <span className="ml-auto flex items-center gap-1 text-[11px] text-ink-3">{draft && !draft.includes("\n") ? <><kbd className="kbd">⇧</kbd><kbd className="kbd">↵</kbd> new line</> : null}</span>
                      <button className="assistant-send" aria-label="Send" title="Send" disabled={(!draft.trim() && !staged.length) || sending || !thread}><Icon name="up" /></button>
                    </div>
                  </div>
                </form>
              )}
            </>
          )}
        </div>
      </section>
    </div>
  );
}

function Centered({ children }: { children: ReactNode }) {
  return <div className="flex min-h-0 flex-1 flex-col items-center justify-center px-8 text-center">{children}</div>;
}

function Account({ st, signing, signIn, signOut }: { st: TtzState | null; signing: boolean; signIn: () => void; signOut: () => void }) {
  if (!st) return null;
  if (signing) {
    return (
      <div className="ttz-account">
        <Icon name="refresh" className="spin !h-4 !w-4 text-cyan" />
        <span className="min-w-0 flex-1 text-[12.5px] leading-tight text-ink-2">Finish signing in in your browser</span>
        <button className="assistant-link" onClick={() => api.ttzCancelSignIn()}>Cancel</button>
      </div>
    );
  }
  if (!st.account) {
    return (
      <div className="ttz-account">
        <span className="min-w-0 flex-1 text-[12px] leading-snug text-ink-3">Anonymous chats need no account.</span>
        <button className="btn sm" onClick={signIn}>Sign in</button>
      </div>
    );
  }
  const a = st.account;
  return (
    <div className="ttz-account">
      {a.avatar?.startsWith("https://") ? <img src={a.avatar} alt="" className="h-7 w-7 rounded-full" referrerPolicy="no-referrer" /> : <span className="ttz-initial">{Array.from(a.username)[0]?.toUpperCase()}</span>}
      <span className="min-w-0 flex-1 truncate text-[13px] font-medium">{a.username}</span>
      <button className="assistant-link" onClick={signOut}>Sign out</button>
    </div>
  );
}

function Messages({ chat, open, messages, anonymousAs, replaced }: { chat: string; open: boolean; messages: TtzMessage[]; anonymousAs?: string | null; replaced: (m: TtzMessage) => void }) {
  let lastDay = "";
  return (
    <>
      {messages.map((m, i) => {
        const d = when(m.at);
        const day = d.toDateString();
        const sep = day !== lastDay;
        lastDay = day;
        const prev = messages[i - 1];
        const grouped = !sep && prev && prev.mine === m.mine && +d - +when(prev.at) < 5 * 60_000;
        return (
          <div key={m.id}>
            {sep && <div className="ttz-day"><span>{dayLabel(d)}</span></div>}
            <div className={`group/msg flex ${m.mine ? "justify-end" : "justify-start"} ${grouped ? "mt-1" : "mt-4"}`}>
              {!m.mine && <img src={profile} alt="" className={`mt-0.5 mr-2.5 h-7 w-7 shrink-0 rounded-full ${grouped ? "invisible" : ""}`} />}
              <div className={`flex max-w-[82%] min-w-0 flex-col ${m.mine ? "items-end" : "items-start"}`}>
                {!grouped && (
                  <span className="mb-1 text-[11.5px] text-ink-3">
                    {m.mine ? (anonymousAs !== undefined ? `You${anonymousAs ? ` · ${anonymousAs}` : ""}` : "You") : <span className="font-semibold text-ink-2">TheHolyOneZ</span>} · {clock(d)}
                  </span>
                )}
                <Message chat={chat} open={open} m={m} replaced={replaced} />
              </div>
            </div>
          </div>
        );
      })}
    </>
  );
}

function Message({ chat, open, m, replaced }: { chat: string; open: boolean; m: TtzMessage; replaced: (m: TtzMessage) => void }) {
  const [editing, setEditing] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  if (m.deleted) return <div className="ttz-locked">{m.deleted === "admin" && m.mine ? "Removed by TheHolyOneZ" : "Message deleted"}</div>;

  async function save() {
    const text = editing?.trim() ?? "";
    if (text === m.text) return setEditing(null);
    if (!text && !m.files.length) return;
    setBusy(true);
    try {
      replaced(await api.ttzEdit(chat, m.id, text));
      setEditing(null);
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  }

  async function remove() {
    const ok = await ask({ title: "Delete this message?", body: "It disappears for TheHolyOneZ too, along with any files in it. This can't be undone.", ok: "Delete", danger: true });
    if (!ok) return;
    try {
      replaced(await api.ttzDelete(chat, m.id));
    } catch (e) {
      fail(e);
    }
  }

  if (editing !== null) {
    return (
      <div className="ttz-edit">
        <textarea className="ttz-input min-h-[76px] resize-y py-2" autoFocus value={editing} aria-label="Edit message" maxLength={20000} onChange={(e) => setEditing(e.target.value)} onPaste={formatPaste}
          onKeyDown={(e) => {
            if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); setEditing(null); return; }
            formatKey(e, () => void save());
          }} />
        <div className="flex justify-end gap-2">
          <button className="btn sm" disabled={busy} onClick={() => setEditing(null)}>Cancel</button>
          <button className="btn sm go" disabled={busy} onClick={() => void save()}>Save</button>
        </div>
      </div>
    );
  }

  return (
    <>
      <div className={`flex max-w-full min-w-0 items-end gap-1.5 ${m.mine ? "flex-row-reverse" : ""}`}>
        <div className={`flex min-w-0 flex-col gap-1.5 ${m.mine ? "items-end" : "items-start"}`}>
          {m.text === null ? (
            <div className="ttz-locked"><Icon name="lock" /> Sealed with a key this launcher doesn't have</div>
          ) : !m.text ? null : m.mine ? (
            <div className="assistant-user selectable break-words"><ChatMarkdown text={m.text} /></div>
          ) : (
            <div className="ttz-thez selectable"><ChatMarkdown text={m.text} /></div>
          )}
          {m.files.map((f) => <FileCard key={f.id} chat={chat} file={f} />)}
          {m.sealedFiles && <div className="ttz-locked"><Icon name="lock" /> Attached files can't be opened with the keys here</div>}
        </div>
        {m.mine && (
          <div className="ttz-actions">
            {open && m.text !== null && <button aria-label="Edit message" title="Edit" onClick={() => setEditing(m.text ?? "")}><Icon name="pen" /></button>}
            <button aria-label="Delete message" title="Delete" onClick={() => void remove()}><Icon name="trash" /></button>
          </div>
        )}
      </div>
      {m.edited && <span className="mt-0.5 text-[11px] text-ink-3">edited</span>}
    </>
  );
}

function FileCard({ chat, file }: { chat: string; file: TtzFile }) {
  const [shown, setShown] = useState<TtzPreview | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [saving, setSaving] = useState<number | null>(null);
  const [saved, setSaved] = useState<string | null>(null);

  const load = useCallback(() => {
    setBusy(true);
    preview(chat, file.id).then(setShown, (e) => setProblem(String(e))).finally(() => setBusy(false));
  }, [chat, file.id]);
  useEffect(() => {
    if (file.shown) load();
  }, [file.shown, load]);

  async function save() {
    setSaving(0);
    setProblem(null);
    const off = await api.onTtzSaving((p) => {
      if (p.id === file.id) setSaving(p.done);
    });
    try {
      setSaved(await api.ttzFileSave(chat, file.id));
    } catch (e) {
      setProblem(String(e));
    } finally {
      off();
      setSaving(null);
    }
  }

  if (!file.kind) {
    return (
      <div className="ttz-file blocked">
        <div className="ttz-file-head">
          <span className="ttz-file-icon"><Icon name="warn" /></span>
          <div className="min-w-0 flex-1"><div className="truncate text-[13px] font-semibold" title={file.name}>{file.name}</div><div className="text-[12px] text-ink-3">Blocked: this kind of file isn't allowed here.</div></div>
        </div>
      </div>
    );
  }
  const previewable = file.kind === "text" || (file.kind === "image" && file.size <= 25 * 1024 * 1024);
  return (
    <div className="ttz-file">
      {shown?.type === "image" && <img src={shown.url} alt={file.name} className="block max-h-[360px] w-full bg-black/40 object-contain" />}
      <div className="ttz-file-head">
        <span className="ttz-file-icon"><Icon name={FILE_ICONS[file.kind]} /></span>
        <div className="min-w-0 flex-1">
          <div className="truncate text-[13px] font-semibold" title={file.name}>{file.name}</div>
          <div className="text-[12px] text-ink-3">{saving !== null ? `Decrypting ${Math.round(saving * 100)}%` : `${FILE_LABELS[file.kind]} · ${fileSize(file.size)}`}</div>
        </div>
        {previewable && !shown && <button className="btn sm" disabled={busy} onClick={load}>{busy ? "Decrypting…" : file.kind === "text" ? "View" : "Show"}</button>}
        {shown?.type === "text" && <button className="btn sm" onClick={() => setShown(null)}>Hide</button>}
        <button className="btn sm" aria-label={`Save ${file.name}`} title="Save to your Downloads folder" disabled={saving !== null} onClick={() => void save()}><Icon name="download" /></button>
      </div>
      {shown?.type === "text" && <pre className="ttz-file-text selectable">{shown.text}{shown.cut ? "\n…" : ""}</pre>}
      {problem && <p className="border-t border-line px-3 py-2 text-[12px] text-bad">{problem}</p>}
      {saved && (
        <p className="flex items-center gap-1.5 border-t border-line px-3 py-1.5 text-[12px] text-ink-3">
          <Icon name="check" className="!h-3.5 !w-3.5 text-ok" /> Saved to Downloads
          <button className="assistant-link ml-auto" onClick={() => api.reveal(saved).catch(fail)}>Show in folder</button>
        </p>
      )}
    </div>
  );
}

function NewChat({ st, signing, signIn, created }: { st: TtzState | null; signing: boolean; signIn: () => void; created: (c: TtzChat) => Promise<void> }) {
  const [kind, setKind] = useState<TtzKind>("HELP");
  const [anonymous, setAnonymous] = useState(!st?.account);
  const [name, setName] = useState(st?.name ?? "");
  const [subject, setSubject] = useState("");
  const [message, setMessage] = useState("");
  const [busy, setBusy] = useState(false);
  const [previewing, setPreviewing] = useState(false);
  const body = useRef<HTMLTextAreaElement>(null);
  const signedIn = !!st?.account;
  useEffect(() => setAnonymous(!signedIn), [signedIn]);
  useEffect(() => setName((n) => n || st?.name || ""), [st?.name]);
  const anon = anonymous || !signedIn;
  const ready = !!subject.trim() && !!message.trim() && !busy && st?.open !== false;

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (!ready) return;
    setBusy(true);
    try {
      const c = await api.ttzCreate({ kind, subject: subject.trim(), message: message.trim(), anonymous: anon, name: anon ? name.trim() : "" });
      setSubject("");
      setMessage("");
      await created(c);
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="scroll min-h-0 flex-1 px-6">
      <form onSubmit={submit} className="mx-auto flex w-full max-w-[640px] flex-col gap-5 py-8">
        <div>
          <img src={ttzIcon} alt="" draggable={false} className="mb-4 h-11 w-11 rounded-xl" />
          <div className="eyebrow mb-3"><b>Talk to TheZ</b> · end-to-end encrypted</div>
          <h3 className="display text-[38px] leading-[1.08]">A direct line to TheHolyOneZ</h3>
          <p className="mt-1.5 max-w-[58ch] text-[13.5px] text-ink-2">Bugs, ideas, requests, or just hi. It goes straight to TheHolyOneZ, the developer of every ZSync tool, encrypted on this computer so only he can read it.</p>
        </div>
        {st?.open === false && <div className="ttz-banner mx-0"><Icon name="warn" /> TalkToZ isn't taking new chats right now. Try again later.</div>}

        <fieldset className="flex flex-col gap-2">
          <legend className="ttz-label">What is it about?</legend>
          <div className="flex flex-wrap gap-2">
            {KINDS.map((k) => (
              <button key={k.value} type="button" className={`ttz-kind ${kind === k.value ? "on" : ""}`} aria-pressed={kind === k.value} onClick={() => setKind(k.value)}>
                <Icon name={k.icon} /> {k.label}
              </button>
            ))}
          </div>
        </fieldset>

        <div className="flex flex-col gap-2">
          <span className="ttz-label">Who is writing?</span>
          {signedIn ? (
            <Segmented value={anon ? "anon" : "me"} onChange={(v) => setAnonymous(v === "anon")}
              options={[{ value: "me", label: `As ${st!.account!.username}` }, { value: "anon", label: "Anonymous" }]} />
          ) : (
            <p className="text-[13px] text-ink-2">
              Anonymously: no account, and not linked to anything else you send.{" "}
              {signing ? <span className="text-ink-3">Signing in…</span> : <button type="button" className="assistant-link -ml-1.5" onClick={signIn}>Sign in instead</button>}
            </p>
          )}
          {anon && (
            <label className="mt-1 flex flex-col gap-1.5">
              <span className="text-[12.5px] text-ink-3">Name, optional. TheHolyOneZ sees it instead of “Anonymous”.</span>
              <input className="ttz-input" value={name} maxLength={40} placeholder="What TheHolyOneZ may call you" onChange={(e) => setName(e.target.value)} autoComplete="nickname" />
            </label>
          )}
        </div>

        <label className="flex flex-col gap-1.5">
          <span className="ttz-label">Subject</span>
          <input className="ttz-input" value={subject} maxLength={255} placeholder="In a few words" onChange={(e) => setSubject(e.target.value)} />
        </label>
        <div className="flex flex-col gap-1.5">
          <span className="ttz-label">Message</span>
          <div className="assistant-composer">
            <FormatBar field={body} preview={previewing} setPreview={setPreviewing} />
            {previewing && <div className="ttz-preview min-h-[140px]">{message.trim() ? <ChatMarkdown text={message} /> : <span className="text-ink-3">Nothing to preview yet.</span>}</div>}
            <textarea ref={body} aria-label="Message" className={`assistant-input min-h-[140px] resize-y ${previewing ? "hidden" : ""}`} value={message} maxLength={20000} placeholder="What's on your mind?" onChange={(e) => setMessage(e.target.value)} onPaste={formatPaste} onKeyDown={(e) => formatKey(e, undefined, () => setPreviewing((p) => !p))} />
          </div>
        </div>

        <div className="flex items-center gap-3">
          <button className="btn go" disabled={!ready}><Icon name="lock" /> {busy ? "Encrypting and sending…" : "Send encrypted"}</button>
          <span className="text-[12px] text-ink-3">{anon ? "Only this launcher can open this chat again." : "Linked to your account."}</span>
        </div>
      </form>
    </div>
  );
}
