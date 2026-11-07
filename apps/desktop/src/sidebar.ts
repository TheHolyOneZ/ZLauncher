export const BLOCKS = ["browse", "favorites", "assistant", "ttz", "installed", "downloads", "settings", "about"] as const;
export type Block = (typeof BLOCKS)[number];

export const BLOCK_LABEL: Record<Block, string> = {
  browse: "Discover",
  favorites: "Favorites",
  assistant: "Assistant",
  ttz: "Talk to TheZ",
  installed: "Library",
  downloads: "Downloads",
  settings: "Settings",
  about: "About",
};

export const ALWAYS_SHOWN: Block = "settings";

const known = (id: string): id is Block => (BLOCKS as readonly string[]).includes(id);

export function blockOrder(saved: string[] | undefined): Block[] {
  const order = (saved ?? []).filter(known).filter((id, i, all) => all.indexOf(id) === i);
  for (const id of BLOCKS) {
    if (order.includes(id)) continue;
    const at = BLOCKS.indexOf(id);
    const after = order.findIndex((o) => BLOCKS.indexOf(o) > at);
    if (after < 0) order.push(id);
    else order.splice(after, 0, id);
  }
  return order;
}

export function blockHidden(hidden: string[] | undefined, id: Block): boolean {
  return id !== ALWAYS_SHOWN && (hidden ?? []).includes(id);
}
