import { useEffect, useState } from "react";
import { AdminStatus, Os, SelfUpdate } from "../api";
import { Icon } from "./Icon";
import logo from "../assets/logo.png";

type Win = {
  minimize: () => Promise<void>;
  toggleMaximize: () => Promise<void>;
  close: () => Promise<void>;
  isMaximized: () => Promise<boolean>;
  onResized: (f: () => void) => Promise<() => void>;
  startResizeDragging: (d: Dir) => Promise<void>;
};
type Dir = "North" | "South" | "East" | "West" | "NorthEast" | "NorthWest" | "SouthEast" | "SouthWest";

const inTauri = "__TAURI_INTERNALS__" in window;
let winPromise: Promise<Win | null> | null = null;
function win(): Promise<Win | null> {
  winPromise ??= inTauri ? import("@tauri-apps/api/window").then((m) => m.getCurrentWindow() as unknown as Win) : Promise.resolve(null);
  return winPromise;
}

function useMaximized(): boolean {
  const [max, setMax] = useState(false);
  useEffect(() => {
    let off: (() => void) | undefined;
    win().then(async (w) => {
      if (!w) return;
      setMax(await w.isMaximized());
      off = await w.onResized(() => {
        w.isMaximized().then(setMax);
      });
    });
    return () => off?.();
  }, []);
  return max;
}

function WinButton({ label, onClick, danger = false, children }: { label: string; onClick: () => void; danger?: boolean; children: React.ReactNode }) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      onClick={onClick}
      className={`grid h-[30px] w-[42px] place-items-center rounded-lg text-ink-3 transition-colors duration-150 ${
        danger ? "hover:bg-[#e5484d] hover:text-white" : "hover:bg-hi hover:text-ink"
      }`}
    >
      <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" aria-hidden>
        {children}
      </svg>
    </button>
  );
}

export function TitleBar({
  os,
  where,
  admin,
  onAdmin,
  update,
  onUpdate,
}: {
  os: Os;
  where: string;
  admin: AdminStatus | null;
  onAdmin: () => void;
  update: SelfUpdate | null;
  onUpdate: () => void;
}) {
  const max = useMaximized();
  const mac = os === "macos";
  return (
    <header data-tauri-drag-region className="relative z-30 flex h-11 shrink-0 items-center border-b border-line bg-side select-none">
      <div data-tauri-drag-region className={`flex h-full w-[248px] shrink-0 items-center gap-2.5 border-r border-line ${mac ? "pl-[84px]" : "pl-4"}`}>
        <img data-tauri-drag-region src={logo} alt="" className="h-[22px] w-[22px] rounded-[6px]" draggable={false} />
        <span data-tauri-drag-region className="font-head text-[15px] font-bold tracking-tight">
          zlauncher
        </span>
      </div>
      <div data-tauri-drag-region className="flex min-w-0 flex-1 items-center px-5">
        <span data-tauri-drag-region className="truncate text-[12.5px] font-medium text-ink-3">
          {where}
        </span>
      </div>
      {update && (
        <button
          className="mr-2 flex h-[26px] items-center gap-1.5 rounded-full border border-cyan/40 bg-cyan/10 px-2.5 text-[11.5px] font-medium text-cyan transition-colors hover:bg-cyan/20"
          title={update.state === "ready" && update.canInstall ? `ZLauncher ${update.version} is downloaded. Click to restart into it.` : `ZLauncher ${update.version} is out`}
          onClick={onUpdate}
          disabled={update.state === "installing"}
        >
          {update.state === "downloading" || update.state === "installing" ? (
            <span className="h-3 w-3 animate-spin rounded-full border-[1.5px] border-cyan/30 border-t-cyan" />
          ) : (
            <Icon name="up" className="!h-3 !w-3" />
          )}
          {update.state === "installing"
            ? "Updating…"
            : update.state === "ready" && update.canInstall
              ? "Restart to update"
              : update.state === "downloading"
                ? `Update ${update.total ? Math.round((update.done / update.total) * 100) : 0}%`
                : `${update.version} available`}
        </button>
      )}
      {admin?.supported && (
        <button
          className={`mr-2 flex h-[26px] items-center gap-1.5 rounded-full border px-2.5 text-[11.5px] font-medium transition-colors ${
            admin.unlocked ? "border-ok/40 bg-ok/10 text-ok hover:bg-ok/15" : "border-line-2 text-ink-3 hover:border-cyan/40 hover:text-ink"
          }`}
          title={admin.unlocked ? "Administrator rights are unlocked for this session. Click to lock." : "Unlock administrator rights for this session"}
          onClick={onAdmin}
        >
          <Icon name="lock" className="!h-3 !w-3" />
          {admin.unlocked ? "Unlocked" : "Locked"}
        </button>
      )}
      {!mac && (
        <div className="flex items-center gap-1 pr-2">
          <WinButton label="Minimize" onClick={() => win().then((w) => w?.minimize())}>
            <path d="M2.5 6h7" />
          </WinButton>
          <WinButton label={max ? "Restore" : "Maximize"} onClick={() => win().then((w) => w?.toggleMaximize())}>
            {max ? (
              <>
                <rect x="2.5" y="4" width="5.5" height="5.5" rx="1" />
                <path d="M4.5 4V3a.5.5 0 0 1 .5-.5h4a.5.5 0 0 1 .5.5v4a.5.5 0 0 1-.5.5H8" />
              </>
            ) : (
              <rect x="2.5" y="2.5" width="7" height="7" rx="1.2" />
            )}
          </WinButton>
          <WinButton label="Close" danger onClick={() => win().then((w) => w?.close())}>
            <path d="M3 3l6 6M9 3l-6 6" />
          </WinButton>
        </div>
      )}
    </header>
  );
}

export function ResizeEdges() {
  const max = useMaximized();
  if (!inTauri || max) return null;
  const edge = (dir: Dir, cls: string, cursor: string) => (
    <div
      key={dir}
      className={`fixed z-[100] ${cls}`}
      style={{ cursor }}
      onMouseDown={(e) => {
        if (e.button === 0) {
          e.preventDefault();
          win().then((w) => w?.startResizeDragging(dir));
        }
      }}
    />
  );
  return (
    <>
      {edge("North", "top-0 right-3 left-3 h-[5px]", "ns-resize")}
      {edge("South", "right-3 bottom-0 left-3 h-[5px]", "ns-resize")}
      {edge("West", "top-3 bottom-3 left-0 w-[5px]", "ew-resize")}
      {edge("East", "top-3 right-0 bottom-3 w-[5px]", "ew-resize")}
      {edge("NorthWest", "top-0 left-0 h-3 w-3", "nwse-resize")}
      {edge("NorthEast", "top-0 right-0 h-3 w-3", "nesw-resize")}
      {edge("SouthWest", "bottom-0 left-0 h-3 w-3", "nesw-resize")}
      {edge("SouthEast", "right-0 bottom-0 h-3 w-3", "nwse-resize")}
    </>
  );
}

export function useSystemFrame(on: boolean, os: Os) {
  useEffect(() => {
    if (os === "macos") return;
    win().then((w) => (w as unknown as { setDecorations?: (b: boolean) => Promise<void> } | null)?.setDecorations?.(on));
  }, [on, os]);
}
