import { ReactNode, useCallback, useEffect, useRef, useState } from "react";
import { AdminStatus, api, ago, AssistantProvider, bytes, canPlayVideo, DeviceCode, GithubStatus, Library, Settings } from "../api";
import { Segmented, Select, Toggle } from "../components/Controls";
import { Icon } from "../components/Icon";
import { ask, fail } from "../components/Overlays";
import assistantIcon from "../assets/assistant.png";

function Section({ icon, img, title, note, children }: { icon?: string; img?: string; title: string; note: string; children: ReactNode }) {
  return (
    <section className="flex flex-col gap-3">
      <div className="flex items-center gap-3 px-1">
        {img ? <img src={img} alt="" draggable={false} className="h-8 w-8 shrink-0" /> : (
          <span className="grid h-8 w-8 shrink-0 place-items-center rounded-[10px] bg-gradient-to-br from-cyan/20 to-violet/20 text-cyan ring-1 ring-cyan/20">
            <Icon name={icon ?? ""} className="!h-[15px] !w-[15px]" />
          </span>
        )}
        <h2 className="text-[17px] font-bold">{title}</h2>
        <span className="truncate text-[13px] text-ink-3">{note}</span>
      </div>
      <div className="card divide-y divide-line overflow-visible">{children}</div>
    </section>
  );
}

function Row({ title, note, children, stack = false, tight = false }: { title: string; note?: ReactNode; children: ReactNode; stack?: boolean; tight?: boolean }) {
  return (
    <div className={`flex ${tight ? "gap-2" : "gap-5"} px-5 py-4 ${stack ? "flex-col" : "items-center"}`}>
      <div className="min-w-0 flex-1">
        <div className="text-[14.5px] font-semibold">{title}</div>
        {note && <div className="mt-0.5 text-[13px] leading-snug text-ink-3">{note}</div>}
      </div>
      <div className={stack ? "" : "shrink-0"}>{children}</div>
    </div>
  );
}

function ThemeCard({ kind, on, pick }: { kind: Settings["theme"]; on: boolean; pick: () => void }) {
  const swatch = (bg: string, card: string, ink: string) => (
    <div className="flex h-full w-full flex-col gap-1.5 p-2.5" style={{ background: bg }}>
      <div className="flex gap-1.5">
        <span className="h-1.5 w-6 rounded-full" style={{ background: ink, opacity: 0.8 }} />
        <span className="h-1.5 w-3 rounded-full" style={{ background: "#3fc6fb" }} />
      </div>
      <div className="grid flex-1 grid-cols-3 gap-1.5">
        {[0, 1, 2].map((i) => (
          <span key={i} className="rounded-[4px]" style={{ background: card }} />
        ))}
      </div>
    </div>
  );
  const dark = swatch("#0b0e16", "#1a2133", "#e9edf5");
  const light = swatch("#f4f1ea", "#fbf9f4", "#1b1f2c");
  return (
    <button
      type="button"
      onClick={pick}
      aria-pressed={on}
      className={`group flex flex-col gap-2 rounded-xl p-1.5 transition-all ${on ? "bg-hi ring-2 ring-cyan" : "ring-1 ring-line hover:ring-line-2"}`}
    >
      <div className="relative aspect-[16/10] w-full overflow-hidden rounded-lg">
        {kind === "system" ? (
          <div className="flex h-full">
            <div className="w-1/2 overflow-hidden">{dark}</div>
            <div className="w-1/2 overflow-hidden">{light}</div>
          </div>
        ) : kind === "dark" ? (
          dark
        ) : (
          light
        )}
      </div>
      <span className={`pb-1 text-[13px] font-medium ${on ? "text-ink" : "text-ink-2"}`}>{kind === "system" ? "Like the system" : kind === "dark" ? "Dark" : "Light"}</span>
    </button>
  );
}

function GithubRows() {
  const [status, setStatus] = useState<GithubStatus | null>(null);
  const [token, setToken] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [code, setCode] = useState<DeviceCode | null>(null);
  const polling = useRef(false);

  const load = () => api.githubStatus().then(setStatus).catch((e) => setError(String(e)));
  useEffect(() => {
    load();
    return () => {
      polling.current = false;
    };
  }, []);

  const saveToken = async () => {
    setBusy(true);
    setError(null);
    try {
      await api.githubSetToken(token);
      setToken("");
      await load();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const signIn = async () => {
    setError(null);
    try {
      const c = await api.githubDeviceStart();
      setCode(c);
      api.openUrl(c.verificationUri);
      polling.current = true;
      let interval = c.interval;
      const deadline = Date.now() + c.expiresIn * 1000;
      while (polling.current && Date.now() < deadline) {
        await new Promise((r) => setTimeout(r, interval * 1000));
        if (!polling.current) return;
        const r = await api.githubDevicePoll(c.deviceCode, interval);
        if (r.status === "done") break;
        if (r.status === "pending") interval = r.interval;
        else throw new Error(r.status === "denied" ? "Sign-in was cancelled on GitHub." : "The code expired. Try again.");
      }
      setCode(null);
      polling.current = false;
      await load();
    } catch (e) {
      setCode(null);
      polling.current = false;
      setError(String(e));
    }
  };

  if (status?.login) {
    return (
      <Row title={`Signed in as ${status.login}`} note="GitHub lookups use your account's limit (5000 an hour).">
        <button className="btn sm" onClick={() => api.githubSignOut().then(load)}>
          Sign out
        </button>
      </Row>
    );
  }
  return (
    <>
      {status?.deviceFlow && (
        <Row title="Sign in with GitHub" note="A code to enter on github.com; no password goes through the launcher.">
          {!code && (
            <button className="btn go" onClick={signIn}>
              Sign in
            </button>
          )}
        </Row>
      )}
      {code && (
        <div className="flex items-center gap-4 px-5 py-4">
          <span className="selectable rounded-xl bg-bg-2 px-4 py-2 font-mono text-[26px] font-medium tracking-[.22em]">{code.userCode}</span>
          <span className="flex-1 text-[13px] text-ink-2">
            Enter this code on the GitHub page that just opened.
            <span className="mt-1 flex items-center gap-1.5 text-ink-3">
              <Icon name="refresh" className="spin !h-3.5 !w-3.5" /> Waiting for GitHub…
            </span>
          </span>
          <button
            className="btn sm"
            onClick={() => {
              polling.current = false;
              setCode(null);
            }}
          >
            Cancel
          </button>
        </div>
      )}
      {!code && (
        <Row
          stack
          title={status?.deviceFlow ? "Or use a token" : "Personal access token"}
          note={
            <>
              Lifts GitHub's limit for anonymous lookups from 60 to 5000 an hour. No scopes needed; it stays on this computer.{" "}
              <button className="text-cyan hover:underline" onClick={() => api.openUrl("https://github.com/settings/personal-access-tokens/new")}>
                Create one
              </button>
            </>
          }
        >
          <div className="flex gap-2">
            <input
              className="input flex-1 font-mono text-[12.5px]"
              type="password"
              placeholder="github_pat_…"
              value={token}
              onChange={(e) => setToken(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && token && saveToken()}
            />
            <button className="btn" disabled={!token || busy} onClick={saveToken}>
              {busy ? "Checking…" : "Save"}
            </button>
          </div>
          {error && <div className="selectable mt-2 text-[13px] text-bad">{error}</div>}
        </Row>
      )}
    </>
  );
}

function BuildCacheRow() {
  const [size, setSize] = useState<number | null>(null);
  const load = () => api.buildCacheSize().then(setSize).catch(() => setSize(null));
  useEffect(() => {
    load();
  }, []);
  return (
    <Row
      title="Build cache"
      note={
        <>
          Sources and compiled output from "Build it yourself". Keeping it makes rebuilds and updates much faster.
          {size !== null && (
            <span className="ml-1.5 rounded-md bg-bg-2 px-1.5 py-0.5 font-mono text-[11.5px] text-ink-2">{bytes(size) || "0 B"}</span>
          )}
        </>
      }
    >
      <button className="btn danger" disabled={!size} onClick={async () => (await ask({ title: "Delete the build cache?", body: "All cloned sources and build output are deleted. The next build from source starts from scratch.", ok: "Delete", danger: true })) && api.clearBuildCache().then(load).catch(fail)}>
        <Icon name="trash" /> Clear
      </button>
    </Row>
  );
}

function AssistantProviderSettings({ settings, save }: { settings: Settings; save: (s: Settings) => void }) {
  const [key, setKey] = useState("");
  const [hasKey, setHasKey] = useState(false);
  const [models, setModels] = useState<string[]>([]);
  const [modelsLoading, setModelsLoading] = useState(false);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  const [modelMessage, setModelMessage] = useState("");
  const [modelOpen, setModelOpen] = useState(false);
  const [showAllModels, setShowAllModels] = useState(true);
  const [modelRefresh, setModelRefresh] = useState(0);
  const provider = settings.assistantProvider;
  useEffect(() => { setHasKey(false); api.assistantKeyStatus(provider).then(setHasKey).catch((e) => setMessage(String(e))); }, [provider]);
  const choose = (next: AssistantProvider) => {
    const defaults: Record<AssistantProvider, [string, string]> = {
      ollama: ["qwen2.5", "http://localhost:11434"],
      openai: ["gpt-5-mini", "https://api.openai.com/v1"],
      anthropic: ["claude-opus-5-5", "https://api.anthropic.com"],
      gemini: ["gemini-3.8-flash", "https://generativelanguage.googleapis.com/v1beta"],
      "openai-compatible": ["", "http://localhost:1234/v1"],
    };
    save({ ...settings, assistantProvider: next, assistantModel: defaults[next][0], assistantBaseUrl: defaults[next][1] });
    setModels([]);
    setKey("");
    setModelMessage("");
    setMessage("");
  };
  const discover = useCallback(async () => {
    setModelsLoading(true);
    setBusy(true);
    setModelMessage("Loading models…");
    try { const found = await api.assistantModels(); setModels(found); setModelMessage(found.length ? `${provider === "anthropic" ? "Suggested" : "Loaded"} ${found.length} model${found.length === 1 ? "" : "s"}.` : "No models were returned by this provider."); }
    catch (e) { setModels([]); setModelMessage(String(e)); }
    finally { setBusy(false); setModelsLoading(false); }
  }, [provider]);
  useEffect(() => {
    const timer = window.setTimeout(() => { void discover(); }, 300);
    return () => window.clearTimeout(timer);
  }, [discover, settings.assistantBaseUrl, modelRefresh]);
  const test = async () => {
    setBusy(true); setMessage("");
    try { setMessage(await api.assistantTest()); }
    catch (e) { setMessage(String(e)); }
    finally { setBusy(false); }
  };
  const saveKey = async () => {
    setBusy(true); setMessage("");
    try { await api.assistantSetKey(provider, key); setKey(""); setHasKey(!!key.trim()); setMessage(key.trim() ? "API key saved on this device." : "Saved API key removed."); setModelRefresh((n) => n + 1); }
    catch (e) { setMessage(String(e)); }
    finally { setBusy(false); }
  };
  const needsKey = provider !== "ollama";
  return <Section img={assistantIcon} title="AI assistant" note="Choose a local or hosted model provider.">
    <Row title="Assistant in the sidebar" note="Show or hide the assistant entry without changing its provider settings.">
      <Toggle on={settings.assistantEnabled} onChange={(v) => save({ ...settings, assistantEnabled: v })} label="Assistant in the sidebar" />
    </Row>
    <Row title="Thinking" note="Let the model reason before it answers and show that reasoning in the chat. Works with models that support it (Claude, Gemini, Qwen 3, DeepSeek R1 and others); slower, and uses more tokens on paid providers.">
      <Toggle on={settings.assistantThinking} onChange={(v) => save({ ...settings, assistantThinking: v })} label="Thinking" />
    </Row>
    <Row title="Provider" note="Ollama and OpenAI-compatible endpoints can run entirely on your computer.">
      <Select value={provider} onChange={choose} options={[
        { value: "ollama", label: "Ollama (local)" }, { value: "openai", label: "OpenAI" },
        { value: "anthropic", label: "Anthropic" }, { value: "gemini", label: "Google Gemini" },
        { value: "openai-compatible", label: "OpenAI-compatible endpoint" },
      ]} />
    </Row>
    <Row stack tight title="Model and endpoint" note="Models load automatically. Type to filter or enter a model name directly.">
      <div className="grid gap-2 sm:grid-cols-2">
        <div className="relative">
          <input className="input w-full pr-10" value={settings.assistantModel} onFocus={() => { setModelOpen(true); setShowAllModels(true); }} onBlur={() => window.setTimeout(() => setModelOpen(false), 120)} onChange={(e) => { save({ ...settings, assistantModel: e.target.value }); setShowAllModels(false); setModelOpen(true); }} placeholder={modelsLoading ? "Loading models…" : "Search or enter model name"} aria-label="Model name" role="combobox" aria-expanded={modelOpen && models.length > 0} aria-autocomplete="list" />
          <button type="button" aria-label="Show available models" className="absolute top-1 right-1 grid h-8 w-8 place-items-center rounded-lg text-ink-3 hover:bg-hi hover:text-ink" onMouseDown={(e) => e.preventDefault()} onClick={() => { setShowAllModels(true); setModelOpen((open) => !open); }}><Icon name="back" className="!h-3.5 !w-3.5 -rotate-90" /></button>
          {modelOpen && models.length > 0 && <div role="listbox" className="menu absolute inset-x-0 top-full z-40 mt-1 max-h-52 overflow-y-auto rounded-xl border border-line-2 bg-card p-1 shadow-[var(--shadow)]">
            {models.filter((m) => showAllModels || !settings.assistantModel || m.toLowerCase().includes(settings.assistantModel.toLowerCase())).slice(0, 50).map((m) => <button key={m} type="button" role="option" aria-selected={settings.assistantModel === m} className="block w-full truncate rounded-lg px-3 py-2 text-left text-[13px] text-ink-2 hover:bg-hi hover:text-ink" onMouseDown={(e) => e.preventDefault()} onClick={() => { save({ ...settings, assistantModel: m }); setModelOpen(false); setShowAllModels(false); }}>{m}</button>)}
            {!showAllModels && !models.some((m) => m.toLowerCase() === settings.assistantModel.toLowerCase()) && <div className="px-3 py-2 text-[11.5px] text-ink-3">Use “{settings.assistantModel}” as a custom model name</div>}
          </div>}
          <div className="mt-1 truncate text-[11.5px] text-ink-3">{modelMessage}</div>
        </div>
        <input className="input" value={settings.assistantBaseUrl} onChange={(e) => save({ ...settings, assistantBaseUrl: e.target.value })} placeholder="Provider base URL" aria-label="Provider base URL" />
      </div>
    </Row>
    {needsKey && <Row stack title="API key" note={provider === "openai-compatible" ? "Optional for local servers. Stored unencrypted in the local ZSync settings database and sent only to this endpoint." : "Stored unencrypted in the local ZSync settings database and sent only to the selected provider."}>
      <div className="flex gap-2"><input className="input flex-1 font-mono" type="password" value={key} onChange={(e) => setKey(e.target.value)} placeholder={hasKey ? "API key saved · enter to replace" : "Paste provider API key"} /><button className="btn" disabled={busy || (!key && !hasKey)} onClick={saveKey}>{key ? "Save key" : "Remove key"}</button></div>
    </Row>}
    <Row title="Check provider" note="Send a short prompt to check the selected model.">
      <button className="btn go" disabled={busy} onClick={test}>{busy ? "Connecting…" : "Test connection"}</button>
    </Row>
    {message && <div className="selectable px-5 pb-4 text-[13px] text-ink-2">{message}</div>}
  </Section>;
}

export function SettingsPage({
  lib,
  settings,
  save,
  admin,
  onAdmin,
}: {
  lib: Library;
  settings: Settings;
  save: (s: Settings) => void;
  admin: AdminStatus | null;
  onAdmin: () => void;
}) {
  const set = <K extends keyof Settings>(k: K, v: Settings[K]) => save({ ...settings, [k]: v });
  const h = lib.host;
  const plugins =
    h.linuxFamily === "debian"
      ? "sudo apt install gstreamer1.0-plugins-good gstreamer1.0-libav"
      : h.linuxFamily === "fedora"
        ? "sudo dnf install gstreamer1-plugins-good gstreamer1-plugin-libav"
        : h.linuxFamily === "suse"
          ? "sudo zypper install gstreamer-plugins-good gstreamer-plugins-libav"
          : "sudo pacman -S gst-plugins-base gst-plugins-good gst-libav";

  return (
    <div className="page mx-auto flex max-w-[860px] flex-col gap-8 px-8 py-8">
      <div>
        <h1 className="text-[32px] font-bold">Settings</h1>
        <p className="text-[14px] text-ink-3">How the launcher looks, installs and keeps your tools up to date.</p>
      </div>

      {!canPlayVideo && h.os === "linux" && (
        <div className="card flex items-start gap-4 border-mark/30 bg-[color-mix(in_srgb,var(--mark)_6%,var(--card))] p-5">
          <span className="grid h-9 w-9 shrink-0 place-items-center rounded-xl bg-mark/15 text-mark">
            <Icon name="play" />
          </span>
          <div className="min-w-0 flex-1 text-[13.5px]">
            <div className="text-[15px] font-semibold">Promo videos are switched off</div>
            <div className="text-ink-2">This system is missing the video plugins the window needs. Install them, then restart the launcher:</div>
            <code className="selectable mt-2 block rounded-lg bg-bg-2 px-3 py-2 font-mono text-[12.5px] text-ink">{plugins}</code>
          </div>
        </div>
      )}

      <Section icon="monitor" title="Appearance" note="Colours and motion.">
        <div className="px-5 py-4">
          <div className="mb-3 text-[14.5px] font-semibold">Theme</div>
          <div className="grid grid-cols-3 gap-3">
            {(["system", "dark", "light"] as const).map((k) => (
              <ThemeCard key={k} kind={k} on={settings.theme === k} pick={() => set("theme", k)} />
            ))}
          </div>
        </div>
        <Row title="Play videos on hover" note="Cards with a promo video play it, muted, while the pointer is on them.">
          <Toggle on={settings.hoverVideo} onChange={(v) => set("hoverVideo", v)} label="Play videos on hover" />
        </Row>
        {h.os !== "macos" && (
          <Row title="Use the system window frame" note="The window border and buttons of your desktop instead of the launcher's own title bar. Handy with tiling window managers.">
            <Toggle on={settings.systemFrame} onChange={(v) => set("systemFrame", v)} label="Use the system window frame" />
          </Row>
        )}
      </Section>

      <AssistantProviderSettings settings={settings} save={save} />

      <Section icon="box" title="Installing" note="What happens when you press Install.">
        {h.os === "linux" && (
          <Row title="Package format" note="A native package is converted for your distribution where needed; the AppImage needs no password.">
            <Segmented
              value={settings.linuxFormat}
              onChange={(v) => set("linuxFormat", v)}
              options={[
                { value: "auto", label: "Native package" },
                { value: "appimage", label: "AppImage first" },
              ]}
            />
          </Row>
        )}
        {admin?.supported && (
          <Row
            title="Administrator rights"
            note={
              admin.unlocked
                ? "Unlocked for this session: installs and removals don't ask for your password. Ends when the launcher closes."
                : "Unlock once for this session instead of a password prompt for every install. Your password goes straight to sudo and is never stored."
            }
          >
            <button className={`btn ${admin.unlocked ? "" : "go"}`} onClick={onAdmin}>
              <Icon name="lock" /> {admin.unlocked ? "Lock" : "Unlock"}
            </button>
          </Row>
        )}
        {admin?.supported && (
          <Row title="Offer it at start-up" note="Ask whether to unlock administrator rights when the launcher starts.">
            <Toggle on={settings.askAdminOnStart} onChange={(v) => set("askAdminOnStart", v)} label="Offer it at start-up" />
          </Row>
        )}
        <Row title="Desktop shortcut" note="Add one for every new install. Each one can be changed in the Library.">
          <Toggle on={settings.desktopShortcut} onChange={(v) => set("desktopShortcut", v)} label="Desktop shortcut" />
        </Row>
        <Row title="Delete installers afterwards" note="Remove the downloaded file once it is installed.">
          <Toggle on={settings.deleteAfterInstall} onChange={(v) => set("deleteAfterInstall", v)} label="Delete installers afterwards" />
        </Row>
      </Section>

      <Section icon="up" title="Updates" note="Finding new tools and new versions.">
        <Row title="Tell me about updates" note="A system notification when a newer version of something you have is out. Each version only once.">
          <Toggle on={settings.notifyUpdates} onChange={(v) => set("notifyUpdates", v)} label="Tell me about updates" />
        </Row>
        <Row title="Check the websites every" note="How old the download information may get before it is looked up again.">
          <Select
            className="w-[150px]"
            value={settings.crawlHours}
            onChange={(v) => set("crawlHours", v)}
            options={[1, 3, 6, 12, 24, 72].map((n) => ({ value: n, label: n === 1 ? "hour" : n < 24 ? `${n} hours` : n === 24 ? "day" : `${n / 24} days` }))}
          />
        </Row>
        <Row title="Check now" note={`Tool list updated ${ago(lib.fetchedAt)}.`}>
          <button className="btn" onClick={() => api.refresh(true)}>
            <Icon name="refresh" className={lib.crawling ? "spin" : ""} /> {lib.crawling ? `Checking ${lib.crawling[0]}/${lib.crawling[1]}` : "Check everything"}
          </button>
        </Row>
      </Section>

      <Section icon="code" title="GitHub" note="Optional. For tools hosted only on GitHub and for builds from source.">
        <GithubRows />
      </Section>

      <Section icon="folder" title="Storage" note="What the launcher keeps on disk.">
        <BuildCacheRow />
        <Row title="Download information" note="Forget what was found on the websites and look everything up again.">
          <button className="btn" onClick={() => api.clearCache()}>
            Look up again
          </button>
        </Row>
      </Section>

    </div>
  );
}
