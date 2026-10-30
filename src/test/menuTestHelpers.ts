import { vi } from "vitest";
import { Menu } from "@tauri-apps/api/menu";

type Entry = {
  kind: string;
  accelerator?: string;
  text?: string;
  item?: string;
  checked?: boolean;
  action?: () => void;
  items?: Entry[];
};

/** The menu bar the app installed last, as the mocked Tauri API recorded it. */
export function lastMenu(): { text: string; items: Entry[] }[] {
  const calls = vi.mocked(Menu.new).mock.calls;
  return (calls[calls.length - 1]?.[0] as unknown as { items: { text: string; items: Entry[] }[] })
    .items;
}

export function entry(menuText: string, entryText: string): Entry {
  const menu = lastMenu().find((m) => m.text === menuText);
  const found = menu?.items.find((e) => e.text === entryText);
  if (!found) throw new Error(`no "${entryText}" in the "${menuText}" menu`);
  return found;
}
