import type { Job, Library, Settings } from "../api";

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
};
let adminUnlocked = false;

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
        current: "0.1.0",
        version: st === "latest" ? null : "0.2.0",
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
