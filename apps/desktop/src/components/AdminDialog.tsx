import { useEffect, useRef, useState } from "react";
import { api } from "../api";
import { Toggle } from "./Controls";
import { Icon } from "./Icon";

export function AdminDialog({
  askOnStart,
  setAskOnStart,
  done,
}: {
  askOnStart: boolean;
  setAskOnStart: (v: boolean) => void;
  done: (unlocked: boolean) => void;
}) {
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const input = useRef<HTMLInputElement>(null);
  const doneRef = useRef(done);
  doneRef.current = done;

  useEffect(() => {
    input.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        doneRef.current(false);
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, []);

  const unlock = async () => {
    if (!password || busy) return;
    setBusy(true);
    setError(null);
    try {
      await api.adminUnlock(password);
      setPassword("");
      done(true);
    } catch (e) {
      setError(String(e));
      setPassword("");
      input.current?.focus();
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="fixed inset-0 z-[80] grid place-items-center bg-[rgba(4,6,12,.72)] p-6" onMouseDown={(e) => e.target === e.currentTarget && done(false)}>
      <div role="dialog" aria-modal="true" aria-labelledby="admin-title" className="page card w-full max-w-[520px] overflow-hidden shadow-[var(--shadow)]">
        <div className="relative p-7 pb-5">
          <div className="glow -top-24 -right-24 h-64 w-64 [--glow:color-mix(in_srgb,var(--cyan)_16%,transparent)]" />
          <span className="relative grid h-12 w-12 place-items-center rounded-2xl bg-gradient-to-br from-cyan/25 to-violet/25 text-cyan ring-1 ring-cyan/25">
            <Icon name="lock" className="!h-5 !w-5" />
          </span>
          <h2 id="admin-title" className="relative mt-4 text-[22px] font-bold">
            Allow installs for this session?
          </h2>
          <p className="relative mt-2 text-[14px] leading-relaxed text-ink-2">
            Installing and removing apps the way your Linux expects (through its package manager) needs administrator rights. Enter your password once
            and the launcher won't have to ask again until you close it.
          </p>
          <ul className="relative mt-4 flex flex-col gap-2 text-[13.5px] text-ink-2">
            {[
              ["shield", "Your password goes straight to sudo, the system's own tool. The launcher never stores it or sends it anywhere."],
              ["box", "It is only used for the installs and removals you start."],
              ["lock", "It ends when you close the launcher, or earlier with the lock in the title bar."],
              ["check", "AppImages never need it. You can also skip this and get the system prompt each time."],
            ].map(([icon, text]) => (
              <li key={text} className="flex gap-2.5">
                <Icon name={icon} className="mt-[3px] !h-[15px] !w-[15px] shrink-0 text-cyan" />
                <span>{text}</span>
              </li>
            ))}
          </ul>
        </div>

        <div className="flex flex-col gap-3 border-t border-line bg-bg-2/60 px-7 py-5">
          <div className="flex gap-2">
            <input
              ref={input}
              type="password"
              className="input flex-1"
              placeholder="Your password"
              autoComplete="current-password"
              value={password}
              disabled={busy}
              onChange={(e) => setPassword(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && unlock()}
            />
            <button className="btn go" disabled={!password || busy} onClick={unlock}>
              {busy ? <span className="h-4 w-4 animate-spin rounded-full border-2 border-[#04131c]/30 border-t-[#04131c]" /> : <Icon name="lock" />}
              Unlock
            </button>
          </div>
          {error && <div className="text-[13px] text-bad">{error}</div>}
          <div className="flex items-center gap-3">
            <label className="flex cursor-pointer items-center gap-2.5 text-[13px] text-ink-3">
              <Toggle on={askOnStart} onChange={setAskOnStart} label="Ask when the launcher starts" />
              Ask when the launcher starts
            </label>
            <button className="btn sm ml-auto" onClick={() => done(false)}>
              Not now
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
