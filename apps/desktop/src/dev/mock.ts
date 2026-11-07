import type { AssistantConversation, AssistantPart, AssistantTurn, Job, Library, Settings, TtzChat, TtzFile, TtzMessage, TtzNewChat, TtzState } from "../api";

const hash = new URLSearchParams(location.hash.slice(1));
let library: Promise<Library> | null = null;

const settings: Settings = {
  crawlHours: 6,
  linuxFormat: "auto",
  desktopShortcut: false,
  theme: (hash.get("theme") as Settings["theme"]) ?? "system",
  deleteAfterInstall: true,
  hoverVideo: true,
  favorites: ["zcashflow", "zhell"],
  sort: "newest",
  notifyUpdates: true,
  systemFrame: false,
  askAdminOnStart: true,
  autoUpdate: true,
  assistantEnabled: true,
  assistantProvider: "ollama",
  assistantModel: "qwen3.5:9b",
  assistantBaseUrl: "http://localhost:11434",
  assistantThinking: true,
  notifyReplies: true,
  assistantTtz: true,
  effects: (hash.get("fx") as Settings["effects"]) ?? "full",
  sidebar: [],
  sidebarHidden: [],
};
let adminUnlocked = false;

const ttzMode = hash.get("ttz") ?? "";
const ago = (minutes: number) => new Date(Date.now() - minutes * 60_000).toISOString().slice(0, 19).replace("T", " ");
const ttzChat = (c: Partial<TtzChat> & Pick<TtzChat, "id" | "subject">): TtzChat => ({ kind: "HELP", status: "OPEN", anonymous: true, name: null, readable: true, unread: false, createdAt: ago(600), lastAt: ago(30), messages: 2, ...c });
const ttzMsg = (id: number, mine: boolean, text: string | null, minutes: number, more: Partial<TtzMessage> = {}): TtzMessage => ({ id, mine, text, at: ago(minutes), edited: false, deleted: null, files: [], sealedFiles: false, ...more });
const ttzFile = (id: string, name: string, size: number, kind: TtzFile["kind"], shown = false): TtzFile => ({ id, name, size, kind, shown });
const ttzPicture = `data:image/svg+xml,${encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="640" height="300"><rect width="640" height="300" fill="#141a26"/><rect x="40" y="40" width="560" height="44" rx="8" fill="#2a3346"/><rect x="40" y="108" width="380" height="16" rx="6" fill="#3a465e"/><rect x="40" y="140" width="460" height="16" rx="6" fill="#3a465e"/><rect x="40" y="172" width="300" height="16" rx="6" fill="#3a465e"/></svg>')}`;
const ttz: TtzState & { threads: Record<string, TtzMessage[]> } = {
  account: ttzMode === "signedin" ? { id: "u1", username: "Etienne", avatar: null } : null,
  name: "Max",
  open: true,
  error: null,
  chats: ttzMode === "empty" ? [] : [
    ttzChat({ id: "c1", subject: "ZType crashes when I paste a long quote", name: "Max", status: "IN_PROGRESS", unread: true, lastAt: ago(4), messages: 4 }),
    ttzChat({ id: "c2", subject: "Idea: a portable mode for ZPad", kind: "REQUEST", anonymous: false, status: "OPEN", lastAt: ago(60 * 26) }),
    ttzChat({ id: "c3", subject: null, readable: false, anonymous: false, kind: "GENERAL", status: "CLOSED", lastAt: ago(60 * 24 * 9) }),
  ],
  threads: {
    c1: [
      ttzMsg(1, true, "Hi! ZType closes right away when I paste a quote longer than about 2,000 characters. Linux, the AppImage, version 0.4.1.", 180, { files: [ttzFile("f1", "crash-screen.png", 412_000, "image", true), ttzFile("f2", "ztype-log.txt", 18_400, "text")] }),
      ttzMsg(2, false, "Thanks for the details, that's exactly what I needed. I can reproduce it: the line wrapper runs out of room on very long words.", 40),
      ttzMsg(3, false, "The fix is in **0.4.2**, out tonight. Here's a build to try:", 39, { edited: true, files: [ttzFile("f3", "ztype-0.4.2-test.zip", 7_340_000, "archive")] }),
      ttzMsg(4, true, null, 20, { deleted: "visitor" }),
      ttzMsg(5, true, "Wow, that was fast. Screen recording of the new build:", 4, { edited: true, files: [ttzFile("f4", "ztype-working.mp4", 31_200_000, "video")] }),
      ttzMsg(6, false, "", 3, { files: [ttzFile("f5", "setup.exe", 900_000, null)] }),
    ],
    c2: [
      ttzMsg(10, true, "Could ZPad run from a USB stick and keep its settings there?", 60 * 26),
      ttzMsg(11, false, "Good idea. I'll put it on the list for the next version.\n\n## What it needs\n\n- [x] **Settings** next to the program\n- [ ] A *portable* flag, see `--portable`\n\n| Part | State |\n| --- | :---: |\n| Paths | done |\n| __Updates__ | ||secret|| |\n\n> Keep it simple.\n\n```rust\nfn portable() -> bool {\n    true\n}\n```\n\nMore at [the site](https://zsync.eu/) or https://zsync.eu/zpad/", 60 * 26 - 5),
    ],
  },
};
try {
  const samples: string[] = JSON.parse(localStorage.getItem("ttz-md-samples") ?? "[]");
  if (samples.length) ttz.threads.c1 = samples.map((text, i) => ttzMsg(100 + i, i % 2 === 0, text, 500 - i));
} catch {
}
const ttzView = (): TtzState => ({ account: ttz.account, name: ttz.name, open: ttz.open, error: ttz.error, chats: structuredClone(ttz.chats) });
const demoChats = hash.get("assistant") === "demo" || hash.has("ask");
let assistantConversations: AssistantConversation[] = [];
const assistantStops = new Set<string>();

function saveAssistantConversations() {
  if (!demoChats) localStorage.setItem("zsync-assistant-chats", JSON.stringify(assistantConversations));
}

const sampleThinking = "The user wants a program to test how fast they type (a few typos in there). In the catalogue, **ZType** is a typing-speed trainer that runs in the terminal, which fits. Let me confirm with a search, then read its details to see whether it runs on this Linux machine.";
const sampleAnswer = "**ZType** is exactly that: a typing-speed trainer that runs right in your terminal.\n\n- 6,000+ real quotes and word lists of up to 10,000 words\n- Live **WPM** and accuracy while you type\n- Works completely offline\n\nThere's no ready-made installer for Linux yet, but I can **build it from source** for you. That takes a few minutes. Want me to start?";
const sampleTools: AssistantPart[] = [
  { kind: "tool", text: "", id: "call_1", name: "search_projects", input: { query: "typing speed test" }, ok: true, summary: "1 match · ZType", output: JSON.stringify({ query: "typing speed test", count: 1, matches: [{ id: "ztype", title: "ZType", category: "Writing & productivity" }] }, null, 2) },
  { kind: "tool", text: "", id: "call_2", name: "get_project_details", input: { project_id: "ztype" }, ok: true, summary: "ZType", output: JSON.stringify({ id: "ztype", title: "ZType", platforms: [], status: { installed: false, canInstall: false, canBuild: true } }, null, 2) },
  { kind: "tool", text: "", id: "call_3", name: "get_project_readme", input: { project_id: "ztype", section: "Features" }, ok: true, summary: "Section “Features”", output: JSON.stringify({ id: "ztype", repo: "TheHolyOneZ/ZTypePy", section: "Features", content: "## Features\n\n- 6,000+ real quotes…" }, null, 2) },
  { kind: "tool", text: "", id: "call_4", name: "search_project_text", input: { project_id: "ztype", pattern: "(\\d[\\d,]*)\\+? (quotes|words)" }, ok: true, summary: "2 matches", output: JSON.stringify({ count: 2, matches: [{ source: "readme", line: 12, match: "6,000+ quotes", groups: ["6,000", "quotes"] }] }, null, 2) },
];

function demoConversations(): AssistantConversation[] {
  const now = Date.now();
  const reply: AssistantTurn = { role: "assistant", content: sampleAnswer, action: "none", projectIds: ["ztype"], model: "qwen3.5:9b", elapsedMs: 14200, parts: [{ kind: "thinking", text: sampleThinking, ms: 6400 }, ...sampleTools, { kind: "text", text: sampleAnswer }] };
  return [
    { id: "demo-1", title: "im searching for a programm wher i cna test how fast i ype", updatedAt: now - 60_000, messages: hash.has("ask") ? [] : [{ role: "user", content: "im searching for a programm wher i cna test how fast i ype" }, reply] },
    { id: "demo-2", title: "Are there updates for my tools?", updatedAt: now - 3 * 3600_000, messages: [{ role: "user", content: "Are there updates for my tools?" }, { role: "assistant", content: "Yes, **Zhell** has an update.", parts: [{ kind: "text", text: "Yes, **Zhell** has an update." }] }] },
    { id: "demo-3", title: "Encrypt files before uploading", updatedAt: now - 2 * 86400_000, messages: [{ role: "user", content: "Encrypt files before uploading" }, { role: "assistant", content: "Try **ZEncrypt**.", parts: [{ kind: "text", text: "Try **ZEncrypt**." }] }] },
  ];
}

assistantConversations = demoChats ? demoConversations() : JSON.parse(localStorage.getItem("zsync-assistant-chats") ?? "[]");

const pause = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function mockStream(chatId: string): Promise<AssistantPart[]> {
  const send = (kind: string, extra: object = {}) => emit("assistant-progress", { chatId, kind, ...extra });
  const stopped = () => assistantStops.has(chatId);
  send("turn_started");
  await pause(500);
  const started = Date.now();
  for (const word of sampleThinking.split(/(?<= )/)) {
    if (stopped()) return [];
    send("thinking", { text: word });
    await pause(70);
  }
  send("thinking_done", { ms: Date.now() - started });
  for (const tool of sampleTools) {
    send("tool_started", { part: { ...tool, ok: undefined, summary: undefined, output: undefined } });
    await pause(700);
    send("tool_finished", { part: tool });
  }
  await pause(300);
  for (const word of sampleAnswer.split(/(?<= )/)) {
    if (stopped()) break;
    send("text", { text: word });
    await pause(60);
  }
  return [{ kind: "thinking", text: sampleThinking, ms: Date.now() - started }, ...sampleTools, { kind: "text", text: sampleAnswer }];
}

type Listener = (e: { payload: unknown }) => void;
const listeners: Record<string, Listener[]> = {};

export function mockListen(event: string, f: Listener): () => void {
  (listeners[event] ??= []).push(f);
  return () => {
    listeners[event] = (listeners[event] ?? []).filter((g) => g !== f);
  };
}

function emit(event: string, payload: unknown) {
  for (const f of listeners[event] ?? []) f({ payload });
}

const base = (id: string, title: string, filename: string, phase: Job["phase"]): Job => ({
  projectId: id,
  operation: phase === "building" ? "build" : phase === "removing" ? "uninstall" : phase === "downloading" ? "download" : "install",
  title,
  filename,
  phase,
  done: 0,
  total: null,
  speed: 0,
  step: null,
  log: [],
  error: null,
  hint: null,
  savedTo: null,
  needsAuth: false,
  missing: [],
  builtFrom: null,
});

const jobs: Job[] =
  hash.get("demo") === "downloads"
    ? [
        { ...base("zcashflow", "ZCashflow", "ZCashflow_1.0.0_linux_amd64.deb", "downloading"), total: 23_200_000 * 4 },
        {
          ...base("zencrypt", "ZEncrypt", "TheHolyOneZ/ZEncrypt", "building"),
          step: "Compiling ZEncrypt (a few minutes the first time)",
          builtFrom: "v0.1.0",
          log: ["   Compiling tauri v2.11.5", "   Compiling zencrypt v0.1.0 (/src-tauri)"],
        },
        { ...base("zpad", "ZPad", "ZPad_0.1.0_amd64.deb", "done"), step: "Installed" },
        { ...base("zircon", "Zircon", "zircon-0.10.0-win64.zip", "done"), savedTo: "/home/you/Downloads/ZLauncher/zircon-0.10.0-win64.zip" },
        { ...base("zdraft", "ZDraft", "ZDraft_0.1.0_amd64.deb", "failed"), error: "The server answered 503", hint: "Try again in a minute." },
      ]
    : [];

if (jobs[0]?.phase === "downloading") {
  const j = jobs[0];
  let t = 0;
  window.setInterval(() => {
    t += 1;
    const target = 9.5e6 + Math.sin(t / 9) * 2.4e6 + Math.sin(t / 2.3) * 0.9e6 + (t % 47 < 4 ? -5e6 : 0);
    j.speed = Math.max(0.4e6, Math.min(target, t * 0.9e6));
    j.done = Math.min(j.total!, j.done + j.speed * 0.15);
    if (j.done >= j.total!) j.done = 0;
    emit("job", { ...j });
  }, 150);
}

export async function mockInvoke(cmd: string, args?: Record<string, unknown>): Promise<unknown> {
  switch (cmd) {
    case "library":
      library ??= fetch("/mock-library.json").then((r) => r.json());
      return library;
    case "settings":
      return settings;
    case "set_settings":
      Object.assign(settings, (args as { settings: Settings }).settings);
      return null;
    case "download":
      return null;
    case "ttz_state":
      return ttzView();
    case "ttz_messages": {
      const { id, since } = args as { id: string; since: number };
      const chat = ttz.chats.find((c) => c.id === id)!;
      chat.unread = false;
      return { messages: structuredClone((ttz.threads[id] ?? []).filter((m) => m.id > since)), changed: [], rev: 0, chat: structuredClone(chat) };
    }
    case "ttz_send": {
      const { id, text, files } = args as { id: string; text: string; files: string[] };
      const list = (ttz.threads[id] ??= []);
      const m = ttzMsg(Math.max(0, ...list.map((x) => x.id)) + 1, true, text, 0, { files: files.map((f) => ttzFile(f, `${f}.png`, 120_000, "image", true)) });
      list.push(m);
      return m;
    }
    case "ttz_edit": {
      const { id, message, text } = args as { id: string; message: number; text: string };
      const m = ttz.threads[id].find((x) => x.id === message)!;
      Object.assign(m, { text, edited: true });
      return structuredClone(m);
    }
    case "ttz_delete": {
      const { id, message } = args as { id: string; message: number };
      const m = ttz.threads[id].find((x) => x.id === message)!;
      Object.assign(m, { text: null, deleted: "visitor", files: [] });
      return structuredClone(m);
    }
    case "ttz_pick_files":
      return [
        { id: "photo", name: "photo.jpg", size: 2_400_000, metadata: false, error: null },
        { id: "clip", name: "screen-recording.mp4", size: 18_000_000, metadata: true, error: null },
      ];
    case "ttz_unstage":
    case "assistant_draft_done":
      return null;
    case "ttz_file_preview": {
      const { file } = args as { file: string };
      if (file === "f2") return { type: "text", text: "[12:04:11] pasting 2412 characters\n[12:04:11] wrap: no room for word of length 2412\n[12:04:11] panic: index out of bounds\n", cut: false };
      return { type: "image", url: ttzPicture };
    }
    case "ttz_file_save":
      return "/home/you/Downloads/ZLauncher/file";
    case "ttz_create": {
      const c = (args as { chat: TtzNewChat }).chat;
      const id = `c${Date.now()}`;
      if (c.anonymous && c.name) ttz.name = c.name;
      const chat = ttzChat({ id, subject: c.subject, kind: c.kind, anonymous: c.anonymous, name: c.anonymous ? c.name || null : null, lastAt: ago(0), createdAt: ago(0), messages: 1 });
      ttz.chats.unshift(chat);
      ttz.threads[id] = [ttzMsg(1, true, c.message, 0)];
      return structuredClone(chat);
    }
    case "ttz_set_name":
      ttz.name = (args as { name: string }).name;
      return null;
    case "ttz_forget":
      ttz.chats = ttz.chats.filter((c) => c.id !== (args as { id: string }).id);
      return null;
    case "ttz_sign_in":
      await new Promise((r) => setTimeout(r, 1500));
      ttz.account = { id: "u1", username: "Etienne", avatar: null };
      return ttz.account;
    case "ttz_cancel_sign_in":
      return null;
    case "ttz_sign_out":
      ttz.account = null;
      return null;
    case "assistant_key_status":
      return false;
    case "assistant_set_key":
      return null;
    case "assistant_models":
      return ["qwen2.5", "qwen3"];
    case "assistant_test":
      return "Mock provider replied successfully.";
    case "assistant_conversations":
      return structuredClone(assistantConversations).sort((a, b) => b.updatedAt - a.updatedAt);
    case "assistant_get_conversation": {
      const chat = assistantConversations.find((item) => item.id === (args as { chatId: string }).chatId);
      if (!chat) throw "This chat is no longer available.";
      return structuredClone(chat);
    }
    case "assistant_new_conversation": {
      const chat: AssistantConversation = { id: crypto.randomUUID(), title: "New chat", messages: [], updatedAt: Date.now() };
      assistantConversations.push(chat);
      saveAssistantConversations();
      return chat;
    }
    case "assistant_delete_conversation":
      assistantConversations = assistantConversations.filter((chat) => chat.id !== (args as { chatId: string }).chatId);
      saveAssistantConversations();
      return null;
    case "assistant_chat": {
      const { chatId, message } = args as { chatId: string; message: string };
      const chat = assistantConversations.find((item) => item.id === chatId);
      if (!chat) throw "This chat was deleted or is no longer available.";
      chat.messages.push({ role: "user", content: message });
      if (chat.title === "New chat") chat.title = message.slice(0, 48);
      assistantStops.delete(chatId);
      const started = Date.now();
      const parts = await mockStream(chatId);
      const stopped = assistantStops.has(chatId);
      const text = parts.filter((p) => p.kind === "text").map((p) => p.text).join("\n\n");
      const ttzDrafts = message.toLowerCase().includes("thez") ? [
        { id: "d1", action: "send" as const, chat: "c1", title: "ZType crashes when I paste a long quote", message: null, kind: null, text: "Thanks! I tried 0.4.2 and the crash is gone, even with very long quotes.", anonymous: true, name: "", who: "Anonymous as Max", done: null },
        { id: "d2", action: "open" as const, chat: null, title: "Idea: a dark theme for ZPad", message: null, kind: "REQUEST" as const, text: "Hi TheHolyOneZ, could ZPad get a dark theme? I mostly write at night.", anonymous: true, name: "Max", who: "Anonymous as Max", done: null },
        { id: "d3", action: "delete" as const, chat: "c1", title: "ZType crashes when I paste a long quote", message: 5, kind: null, text: "Wow, that was fast. Screen recording of the new build:", anonymous: true, name: "", who: "Anonymous as Max", done: null },
      ] : [];
      const reply = { ttzDrafts, text, projectIds: stopped ? [] : ["ztype"], artifactUrls: [], action: "none" as const, parts, model: settings.assistantModel, elapsedMs: Date.now() - started, stopped };
      chat.messages.push({ role: "assistant", content: text, ...reply });
      chat.updatedAt = Date.now();
      saveAssistantConversations();
      return reply;
    }
    case "assistant_stop":
      assistantStops.add((args as { chatId: string }).chatId);
      return null;
    case "jobs":
      return jobs;
    case "build_requirements":
      return [];
    case "build_cache_size":
      return 3_400_000_000;
    case "admin_status":
      return { supported: !location.hash.includes("noadmin"), unlocked: adminUnlocked };
    case "admin_unlock":
      if ((args as { password: string }).password !== "demo") throw "Wrong password.";
      adminUnlocked = true;
      return null;
    case "admin_lock":
      adminUnlocked = false;
      return null;
    case "self_update_status": {
      const st = new URLSearchParams(location.hash.slice(1)).get("selfupdate") ?? "latest";
      const manual = st === "manual";
      return {
        state: manual ? "available" : st,
        current: "0.4.0",
        version: st === "latest" ? null : "0.4.0",
        notes: "Faster start-up, a new About page and updates that install themselves.",
        done: 31_000_000,
        total: 52_000_000,
        error: st === "error" ? "Could not reach zsync.eu." : null,
        canInstall: !manual,
        reason: manual ? "This copy came from a package your system doesn't install directly." : null,
      };
    }
    case "github_status":
      return { login: null, deviceFlow: true };
    default:
      return null;
  }
}
