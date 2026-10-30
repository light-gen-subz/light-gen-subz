import { CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu } from "@tauri-apps/api/menu";

/** A menu entry, described as data so it can be rebuilt (language, theme, recent files). */
export type MenuSpec =
  | { kind: "item"; text: string; accelerator?: string; enabled?: boolean; action: () => void }
  | { kind: "check"; text: string; checked: boolean; action: () => void }
  | { kind: "separator" }
  | {
      kind: "predefined";
      item:
        | "Quit"
        | "Undo"
        | "Redo"
        | "Cut"
        | "Copy"
        | "Paste"
        | "SelectAll"
        | "Hide"
        | "HideOthers"
        | "ShowAll";
      text?: string;
    }
  | { kind: "submenu"; text: string; enabled?: boolean; items: MenuSpec[] };

export interface TopMenu {
  text: string;
  items: MenuSpec[];
}

type Run = (path: string) => void;

type Built = MenuItem | CheckMenuItem | PredefinedMenuItem | Submenu;

async function build(spec: MenuSpec, path: string, run: Run): Promise<Built> {
  switch (spec.kind) {
    case "separator":
      return PredefinedMenuItem.new({ item: "Separator" });
    case "predefined":
      return PredefinedMenuItem.new({ item: spec.item, text: spec.text });
    case "item":
      return MenuItem.new({
        text: spec.text,
        accelerator: spec.accelerator,
        enabled: spec.enabled,
        action: () => run(path),
      });
    case "check":
      return CheckMenuItem.new({ text: spec.text, checked: spec.checked, action: () => run(path) });
    case "submenu":
      return Submenu.new({
        text: spec.text,
        enabled: spec.enabled,
        items: await Promise.all(spec.items.map((s, i) => build(s, `${path}.${i}`, run))),
      });
  }
}

/** Installs `menus` as the application menu. An entry calls `run` with its position
 * ("0.2" is the third entry of the first menu), so the caller always runs its latest code. */
export async function installMenu(menus: TopMenu[], run: Run): Promise<void> {
  const top = await Promise.all(
    menus.map(async (m, i) =>
      Submenu.new({
        text: m.text,
        items: await Promise.all(m.items.map((s, j) => build(s, `${i}.${j}`, run))),
      }),
    ),
  );
  const menu = await Menu.new({ items: top });
  await menu.setAsAppMenu();
}

/** The entry at a position written by `installMenu`. */
export function findSpec(menus: TopMenu[], path: string): MenuSpec | undefined {
  const [first, ...rest] = path.split(".").map(Number);
  let items: MenuSpec[] | undefined = menus[first]?.items;
  let spec: MenuSpec | undefined;
  for (const index of rest) {
    spec = items?.[index];
    items = spec?.kind === "submenu" ? spec.items : undefined;
  }
  return spec;
}
