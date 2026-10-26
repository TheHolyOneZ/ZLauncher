import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen, UnlistenFn } from "@tauri-apps/api/event";

const inTauri = "__TAURI_INTERNALS__" in window;

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (inTauri) return tauriInvoke<T>(cmd, args);
  if (!import.meta.env.DEV) throw new Error("not running inside ZLauncher");
  const { mockInvoke } = await import("./dev/mock");
  return mockInvoke(cmd, args) as Promise<T>;
}

function listen<T>(event: string, f: (e: { payload: T }) => void): Promise<UnlistenFn> {
  if (inTauri) return tauriListen<T>(event, f);
  if (!import.meta.env.DEV) return Promise.resolve(() => {});
  return import("./dev/mock").then((m) => m.mockListen(event, f as (e: { payload: unknown }) => void));
}

export type Os = "windows" | "linux" | "macos" | "android" | "ios" | "browser";
export type Arch = "x64" | "arm64" | "universal";
export type Kind =
  | "deb" | "rpm" | "app-image" | "pacman" | "msi" | "setup" | "portable-exe" | "dmg"
  | "app-archive" | "zip" | "tar-gz" | "binary" | "apk" | "ipa" | "extension";

export interface Link { label: string; url: string; primary: boolean }
export interface Video { mp4: string; poster: string | null; duration: number | null }

export interface Artifact {
  project_id: string;
  url: string;
  filename: string;
  os: Os;
  arch: Arch | null;
  kind: Kind;
  version: string | null;
  size: number | null;
  sha256: string | null;
  source: string;
}

export interface Installed {
  version: string | null;
  method: string;
  filename: string;
  installedAt: number;
  present: boolean;
  desktopShortcut: boolean;
  update: string | null;
  external: string | null;
}

export interface Project {
  id: string;
  title: string;
  category: string | null;
  description: string;
  url: string | null;
  links: Link[];
  categories: string[];
  added: string | null;
  retired: boolean;
  video: Video | null;
  artifacts: Artifact[];
  version: string | null;
  releaseNotes: string | null;
  released: string | null;
  crawledAt: number | null;
  recommended: string | null;
  installed: Installed | null;
}

export interface Host { os: Os; arch: Arch; linuxFamily: string | null; name: string; video: boolean }

export interface Library {
  host: Host;
  projects: Project[];
  fetchedAt: number | null;
  crawling: [number, number] | null;
  catalogueError: string | null;
}

export type Phase = "queued" | "building" | "downloading" | "installing" | "removing" | "done" | "failed" | "cancelled";

export interface Missing { name: string; why: string; install: string }

export interface GithubStatus { login: string | null; deviceFlow: boolean }

export interface DeviceCode { deviceCode: string; userCode: string; verificationUri: string; interval: number; expiresIn: number }

export type DevicePoll = { status: "pending"; interval: number } | { status: "done"; login: string } | { status: "expired" } | { status: "denied" };

export interface Job {
  projectId: string;
  operation: "install" | "download" | "build" | "uninstall";
  title: string;
  filename: string;
  phase: Phase;
  done: number;
  total: number | null;
  speed: number;
  step: string | null;
  log: string[];
  error: string | null;
  hint: string | null;
  savedTo: string | null;
  needsAuth: boolean;
  missing: Missing[];
  builtFrom: string | null;
}

export interface Settings {
  crawlHours: number;
  linuxFormat: "auto" | "appimage";
  desktopShortcut: boolean;
  theme: "system" | "dark" | "light";
  deleteAfterInstall: boolean;
  hoverVideo: boolean;
  favorites: string[];
  sort: "newest" | "name" | "here";
  notifyUpdates: boolean;
  systemFrame: boolean;
  askAdminOnStart: boolean;
  autoUpdate: boolean;
  assistantEnabled: boolean;
  assistantProvider: AssistantProvider;
  assistantModel: string;
  assistantBaseUrl: string;
  assistantThinking: boolean;
}

export type AssistantProvider = "ollama" | "openai" | "anthropic" | "gemini" | "openai-compatible";
export type AssistantAction = "none" | "install" | "download" | "update" | "update_now" | "uninstall" | "launch" | "build";
export interface AssistantPart {
  kind: "thinking" | "text" | "tool" | "error";
  text: string;
  ms?: number;
  id?: string;
  name?: string;
  input?: Record<string, unknown>;
  output?: string;
  summary?: string;
  ok?: boolean;
}
export interface AssistantStep { name: string; input: Record<string, unknown>; output: string; ok: boolean }
export interface AssistantProgress {
  chatId: string;
  kind: "turn_started" | "thinking" | "thinking_done" | "text" | "tool_started" | "tool_finished" | "error";
  text?: string;
  part?: AssistantPart;
  ms?: number;
}
export interface AssistantTurn {
  role: "user" | "assistant";
  content: string;
  projectIds?: string[];
  action?: AssistantAction;
  artifactUrls?: (string | null)[];
  parts?: AssistantPart[];
  model?: string;
  elapsedMs?: number;
  stopped?: boolean;
  steps?: AssistantStep[];
  activity?: string[];
}
export interface AssistantConversation { id: string; title: string; messages: AssistantTurn[]; updatedAt: number }
export interface AssistantReply { text: string; projectIds: string[]; artifactUrls: (string | null)[]; action: AssistantAction; parts: AssistantPart[]; model: string; elapsedMs: number; stopped: boolean }

export interface SelfUpdate {
  state: "idle" | "checking" | "latest" | "available" | "downloading" | "ready" | "installing" | "error";
  current: string;
  version: string | null;
  notes: string | null;
  done: number;
  total: number | null;
  error: string | null;
  canInstall: boolean;
  reason: string | null;
}

export interface AdminStatus { supported: boolean; unlocked: boolean }

export const api = {
  library: () => invoke<Library>("library"),
  jobs: () => invoke<Job[]>("jobs"),
  refresh: (force: boolean) => invoke<void>("refresh", { force }),
  install: (projectId: string, url?: string) => invoke<void>("install", { projectId, url: url ?? null }),
  download: (projectId: string, url?: string) => invoke<void>("download", { projectId, url: url ?? null }),
  uninstall: (projectId: string) => invoke<void>("uninstall", { projectId }),
  cancel: (projectId: string) => invoke<void>("cancel", { projectId }),
  clearJob: (projectId: string) => invoke<void>("clear_job", { projectId }),
  launch: (projectId: string) => invoke<void>("launch", { projectId }),
  setDesktopShortcut: (projectId: string, want: boolean) => invoke<void>("set_desktop_shortcut", { projectId, want }),
  forget: (projectId: string) => invoke<void>("forget", { projectId }),
  settings: () => invoke<Settings>("settings"),
  setSettings: (settings: Settings) => invoke<void>("set_settings", { settings }),
  assistantKeyStatus: (provider: AssistantProvider) => invoke<boolean>("assistant_key_status", { provider }),
  assistantSetKey: (provider: AssistantProvider, key: string) => invoke<void>("assistant_set_key", { provider, key }),
  assistantConversations: () => invoke<AssistantConversation[]>("assistant_conversations"),
  assistantGetConversation: (chatId: string) => invoke<AssistantConversation>("assistant_get_conversation", { chatId }),
  assistantNewConversation: () => invoke<AssistantConversation>("assistant_new_conversation"),
  assistantDeleteConversation: (chatId: string) => invoke<void>("assistant_delete_conversation", { chatId }),
  assistantChat: (chatId: string, message: string, projects: { id: string; title: string; description: string; category: string | null; url: string | null; recommended: string | null; links: Project["links"]; video: Project["video"]; artifacts: Artifact[]; version: string | null; released: string | null; releaseNotes: string | null; installed: boolean; installedVersion: string | null; update: string | null; installable: boolean; removable: boolean }[]) => invoke<AssistantReply>("assistant_chat", { chatId, message, projects }),
  assistantStop: (chatId: string) => invoke<void>("assistant_stop", { chatId }),
  onAssistantProgress: (f: (event: AssistantProgress) => void) => listen<AssistantProgress>("assistant-progress", (e) => f(e.payload)),
  assistantTest: () => invoke<string>("assistant_test"),
  assistantModels: () => invoke<string[]>("assistant_models"),
  clearCache: () => invoke<void>("clear_cache"),
  openUrl: (url: string) => invoke<void>("open_url", { url }),
  reveal: (path: string) => invoke<void>("reveal", { path }),
  build: (projectId: string) => invoke<void>("build", { projectId }),
  buildRequirements: () => invoke<Missing[]>("build_requirements"),
  buildCacheSize: () => invoke<number>("build_cache_size"),
  clearBuildCache: () => invoke<void>("clear_build_cache"),
  githubStatus: () => invoke<GithubStatus>("github_status"),
  githubSetToken: (token: string) => invoke<string>("github_set_token", { token }),
  githubSignOut: () => invoke<void>("github_sign_out"),
  githubDeviceStart: () => invoke<DeviceCode>("github_device_start"),
  githubDevicePoll: (deviceCode: string, interval: number) => invoke<DevicePoll>("github_device_poll", { deviceCode, interval }),
  adminStatus: () => invoke<AdminStatus>("admin_status"),
  adminUnlock: (password: string) => invoke<void>("admin_unlock", { password }),
  adminLock: () => invoke<void>("admin_lock"),
  selfUpdateStatus: () => invoke<SelfUpdate>("self_update_status"),
  selfUpdateCheck: () => invoke<void>("self_update_check"),
  selfUpdateInstall: () => invoke<void>("self_update_install"),
  onSelfUpdate: (f: (u: SelfUpdate) => void) => listen<SelfUpdate>("self-update", (e) => f(e.payload)),
  onLibrary: (f: () => void) => listen("library", f),
  onJob: (f: (job: Job) => void) => listen<Job>("job", (e) => f(e.payload)),
};

export const OS_LABEL: Record<Os, string> = {
  windows: "Windows",
  linux: "Linux",
  macos: "macOS",
  android: "Android",
  ios: "iOS",
  browser: "Browser",
};

const KIND_LABEL: Record<Kind, string> = {
  deb: ".deb", rpm: ".rpm", "app-image": "AppImage", pacman: "Arch package", msi: ".msi",
  setup: "Installer", "portable-exe": "Portable .exe", dmg: ".dmg", "app-archive": ".app archive",
  zip: ".zip", "tar-gz": ".tar.gz", binary: "Binary", apk: ".apk", ipa: ".ipa", extension: "Extension",
};

export const kindLabel = (k: Kind) => KIND_LABEL[k];

export function installable(kind: Kind, os: Os): boolean {
  if (os === "linux") return kind === "deb" || kind === "rpm" || kind === "app-image";
  if (os === "windows") return kind === "setup" || kind === "msi" || kind === "portable-exe";
  if (os === "macos") return kind === "dmg" || kind === "app-archive";
  return false;
}

export function bytes(n: number | null | undefined): string {
  if (n == null) return "";
  if (n >= 1 << 30) return `${(n / (1 << 30)).toFixed(1)} GB`;
  if (n >= 1 << 20) return `${(n / (1 << 20)).toFixed(1)} MB`;
  if (n >= 1 << 10) return `${Math.round(n / 1024)} KB`;
  return `${n} B`;
}

export function rate(bytesPerSecond: number): string {
  if (bytesPerSecond >= 1 << 20) return `${(bytesPerSecond / (1 << 20)).toFixed(1)} MB/s`;
  if (bytesPerSecond >= 1 << 10) return `${Math.round(bytesPerSecond / 1024)} KB/s`;
  return `${Math.round(bytesPerSecond)} B/s`;
}

export function eta(done: number, total: number | null, bytesPerSecond: number, suffix = true): string | null {
  if (!total || bytesPerSecond <= 0 || done >= total) return null;
  const s = Math.ceil((total - done) / bytesPerSecond);
  const t = s < 60 ? `${s} s` : s < 3600 ? `${Math.floor(s / 60)} min ${String(s % 60).padStart(2, "0")} s` : `${Math.floor(s / 3600)} h ${Math.floor((s % 3600) / 60)} min`;
  return suffix ? `${t} left` : t;
}

export function ago(unix: number | null): string {
  if (!unix) return "never";
  const s = Math.max(0, Date.now() / 1000 - unix);
  if (s < 60) return "just now";
  if (s < 3600) return `${Math.floor(s / 60)} min ago`;
  if (s < 86400) return `${Math.floor(s / 3600)} h ago`;
  return `${Math.floor(s / 86400)} d ago`;
}

export function osesOf(p: Project): Os[] {
  const order: Os[] = ["windows", "linux", "macos", "android", "ios", "browser"];
  const have = new Set(p.artifacts.map((a) => a.os));
  return order.filter((o) => have.has(o));
}

export function summary(p: Project): string {
  const m = p.description.match(/^.{20,}?[.!?](\s|$)/);
  return (m ? m[0] : p.description).trim();
}

export let canPlayVideo = false;

const webviewCanPlay: boolean = (() => {
  try {
    const v = document.createElement("video");
    return v.canPlayType('video/mp4; codecs="avc1.42E01E"') !== "";
  } catch {
    return false;
  }
})();

export function setVideoSupport(backendSaysOk: boolean) {
  const off = import.meta.env.DEV && new URLSearchParams(location.hash.slice(1)).has("novideo");
  canPlayVideo = backendSaysOk && webviewCanPlay && !off;
}
