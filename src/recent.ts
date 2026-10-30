const KEY = "light-gen-subz:recent";
const MAX = 5;

export type RecentEntry = { path: string; name: string; dir: string };

export function toEntry(path: string): RecentEntry {
  const cut = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return { path, name: path.slice(cut + 1), dir: cut > 0 ? path.slice(0, cut) : "" };
}

/** The files opened lately, most recent first. */
export function loadRecent(): RecentEntry[] {
  try {
    const raw: unknown = JSON.parse(localStorage.getItem(KEY) ?? "[]");
    if (!Array.isArray(raw)) return [];
    return raw
      .filter((p): p is string => typeof p === "string" && p.length > 0)
      .slice(0, MAX)
      .map(toEntry);
  } catch {
    return [];
  }
}

function save(list: RecentEntry[]): RecentEntry[] {
  try {
    localStorage.setItem(KEY, JSON.stringify(list.map((e) => e.path)));
  } catch {
    // Storage blocked: the list is just not remembered.
  }
  return list;
}

export function addRecent(list: RecentEntry[], path: string): RecentEntry[] {
  return save([toEntry(path), ...list.filter((e) => e.path !== path)].slice(0, MAX));
}

export function removeRecent(list: RecentEntry[], path: string): RecentEntry[] {
  return save(list.filter((e) => e.path !== path));
}

export function clearRecent(): RecentEntry[] {
  return save([]);
}
