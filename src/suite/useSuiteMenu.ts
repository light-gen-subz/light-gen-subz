import { useEffect, useMemo, useRef, useState } from "react";
import { exit } from "@tauri-apps/plugin-process";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { findSpec, installMenu, type MenuSpec, type TopMenu } from "./menu";
import { applyTheme, loadTheme, saveTheme, type Theme } from "./theme";
import { useSuite } from "./UpdaterProvider";
import type { SuiteLang, SuiteT } from "./messages";

interface Options {
  t: SuiteT;
  lang: SuiteLang;
  appName: string;
  onLanguage: (lang: SuiteLang) => void;
  /** The entries of the File menu, before "Quit". */
  file: MenuSpec[];
  /** Menus of the app's own, between View and Help. */
  extra?: TopMenu[];
  /** Entries of an Edit menu of the app's own (undo, redo…). Without it there is no Edit menu,
   * except on macOS where text fields need one for copy and paste. */
  edit?: MenuSpec[];
  /** Entries added to Help after What's new (keyboard shortcuts…). */
  helpExtra?: MenuSpec[];
  /** What "Check for updates" does besides looking for the app's own update. */
  onCheckUpdates?: () => void;
}

const isMac = () => /Mac/i.test(navigator.userAgent);

/** The menu bar every app of the suite has: File, Edit, View, Help. The app supplies what is
 * its own (File entries, extra menus); the rest (Edit, theme, full screen, About, What's
 * new, updates, language) is the same everywhere. */
export function useSuiteMenu({
  t,
  lang,
  appName,
  onLanguage,
  file,
  extra = [],
  edit,
  helpExtra = [],
  onCheckUpdates,
}: Options) {
  const { checkNow, openAbout, openChangelog } = useSuite();
  const [theme, setTheme] = useState<Theme>(loadTheme);

  useEffect(() => {
    saveTheme(theme);
    applyTheme(theme);
    void getCurrentWindow()
      .setTheme(theme === "system" ? null : theme)
      .catch(() => {});
  }, [theme]);

  const menus = useMemo<TopMenu[]>(() => {
    const toggleFullscreen = async () => {
      const win = getCurrentWindow();
      await win.setFullscreen(!(await win.isFullscreen()));
    };
    const themeItem = (value: Theme, key: string): MenuSpec => ({
      kind: "check",
      text: t(key),
      checked: theme === value,
      action: () => setTheme(value),
    });
    const langItem = (value: SuiteLang, text: string): MenuSpec => ({
      kind: "check",
      text,
      checked: lang === value,
      action: () => onLanguage(value),
    });
    const mac = isMac();
    // A predefined Quit item is greyed out on Linux: it is an ordinary entry that exits.
    const quit: MenuSpec = {
      kind: "item",
      text: t("menu.quit"),
      accelerator: "CmdOrCtrl+Q",
      action: () => void exit(0),
    };
    const bar: TopMenu[] = [
      ...(mac
        ? [
            {
              text: appName,
              items: [
                { kind: "predefined", item: "Hide" },
                { kind: "predefined", item: "HideOthers" },
                { kind: "predefined", item: "ShowAll" },
                { kind: "separator" },
                quit,
              ] as MenuSpec[],
            },
          ]
        : []),
      {
        text: t("menu.file"),
        items: [
          ...file,
          ...(file.length ? ([{ kind: "separator" }] as MenuSpec[]) : []),
          ...(mac ? [] : [quit]),
        ],
      },
      // Copy and paste work in text fields without it, except on macOS.
      ...(edit || mac
        ? [
            {
              text: t("menu.edit"),
              items: [
                ...(edit ?? [
                  { kind: "predefined", item: "Undo", text: t("menu.undo") },
                  { kind: "predefined", item: "Redo", text: t("menu.redo") },
                ]),
                ...(mac
                  ? ([
                      { kind: "separator" },
                      { kind: "predefined", item: "Cut", text: t("menu.cut") },
                      { kind: "predefined", item: "Copy", text: t("menu.copy") },
                      { kind: "predefined", item: "Paste", text: t("menu.paste") },
                      { kind: "predefined", item: "SelectAll", text: t("menu.selectAll") },
                    ] as MenuSpec[])
                  : []),
              ] as MenuSpec[],
            },
          ]
        : []),
      {
        text: t("menu.view"),
        items: [
          {
            kind: "item",
            text: t("menu.fullscreen"),
            accelerator: mac ? "Ctrl+Cmd+F" : "F11",
            action: () => void toggleFullscreen(),
          },
          {
            kind: "submenu",
            text: t("menu.theme"),
            items: [
              themeItem("system", "menu.themeSystem"),
              themeItem("light", "menu.themeLight"),
              themeItem("dark", "menu.themeDark"),
            ],
          },
        ],
      },
      ...extra,
      {
        text: t("menu.help"),
        items: [
          { kind: "item", text: t("menu.about", { name: appName }), action: openAbout },
          { kind: "item", text: t("cl.open"), action: openChangelog },
          ...helpExtra,
          { kind: "separator" },
          {
            kind: "item",
            text: t("menu.checkUpdates"),
            action: () => {
              onCheckUpdates?.();
              void checkNow();
            },
          },
          { kind: "separator" },
          {
            kind: "submenu",
            text: t("menu.language"),
            items: [langItem("en", "English"), langItem("fr", "Français")],
          },
        ],
      },
    ];
    return bar;
  }, [
    t,
    lang,
    theme,
    appName,
    onLanguage,
    file,
    extra,
    edit,
    helpExtra,
    onCheckUpdates,
    checkNow,
    openAbout,
    openChangelog,
  ]);

  // Rebuilt only when something visible changes: the entries' actions are looked up in the
  // latest `menus` when they are clicked, so they never go stale.
  const latest = useRef(menus);
  useEffect(() => {
    latest.current = menus;
  }, [menus]);
  const signature = JSON.stringify(menus);
  useEffect(() => {
    void installMenu(latest.current, (path) => {
      const spec = findSpec(latest.current, path);
      if (spec && "action" in spec) spec.action();
    }).catch(() => {
      // No native menu (a plain browser, a test): the app works without it.
    });
  }, [signature]);
}
