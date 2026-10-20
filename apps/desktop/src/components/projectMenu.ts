import { api, installable, Job, Library, Project } from "../api";
import { isActive } from "../state";
import { ask, fail, MenuItem } from "./Overlays";

export async function uninstall(p: Project) {
  const ok = await ask({
    title: `Uninstall ${p.title}?`,
    body: "It is removed from this computer, with its menu entry and shortcuts. Your own files made with it stay.",
    ok: "Uninstall",
    danger: true,
  });
  if (ok) api.uninstall(p.id).catch(fail);
}

export function copyText(text: string) {
  navigator.clipboard.writeText(text).catch(fail);
}

export function projectMenu(
  p: Project,
  lib: Library,
  job: Job | undefined,
  o: { open?: () => void; fav?: boolean; toggleFav?: () => void },
): MenuItem[] {
  const inst = p.installed;
  const busy = isActive(job);
  const rec = p.artifacts.find((a) => a.url === p.recommended);
  const here = p.artifacts.find((a) => a.os === lib.host.os && !installable(a.kind, lib.host.os));
  const items: MenuItem[] = [];

  if (o.open) items.push({ label: "Show details", icon: "info", run: o.open });
  if (busy) {
    if (job!.phase === "downloading" || job!.phase === "building") items.push({ label: "Stop", icon: "x", run: () => api.cancel(p.id) });
  } else if (inst?.present) {
    items.push({ label: "Open", icon: "play", run: () => api.launch(p.id).catch(fail) });
    if (inst.update) items.push({ label: `Update to ${inst.update}`, icon: "up", run: () => api.install(p.id).catch(fail) });
  } else if (rec) {
    items.push({ label: "Install", icon: "download", run: () => api.install(p.id).catch(fail) });
  } else if (here) {
    items.push({ label: "Download", icon: "download", run: () => api.install(p.id, here.url).catch(fail) });
  }

  items.push("-");
  if (o.toggleFav) items.push({ label: o.fav ? "Remove from favorites" : "Add to favorites", icon: "heart", run: o.toggleFav });
  if (inst?.present && !inst.external && !busy) {
    items.push({
      label: inst.desktopShortcut ? "Remove desktop shortcut" : "Add desktop shortcut",
      icon: "monitor",
      run: () => api.setDesktopShortcut(p.id, !inst.desktopShortcut).catch(fail),
    });
  }
  if (p.url) {
    const url = p.url;
    items.push({ label: "Open website", icon: "globe", run: () => api.openUrl(url).catch(fail) });
    items.push({ label: "Copy website link", icon: "copy", run: () => copyText(url) });
  }

  if (inst && !busy) {
    items.push("-");
    if (inst.present && !inst.external) items.push({ label: "Uninstall…", icon: "trash", danger: true, run: () => uninstall(p) });
    else if (!inst.present) items.push({ label: "Forget", icon: "x", run: () => api.forget(p.id).catch(fail) });
  }
  return items.filter((it, i, all) => it !== "-" || (i > 0 && i < all.length - 1 && all[i - 1] !== "-"));
}
