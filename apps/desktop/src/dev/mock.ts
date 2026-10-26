import type { AssistantConversation, AssistantPart, AssistantTurn, Job, Library, Settings } from "../api";

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
};
let adminUnlocked = false;
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
      const reply = { text, projectIds: stopped ? [] : ["ztype"], artifactUrls: [], action: "none" as const, parts, model: settings.assistantModel, elapsedMs: Date.now() - started, stopped };
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
        current: "0.2.0",
        version: st === "latest" ? null : "0.3.0",
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
