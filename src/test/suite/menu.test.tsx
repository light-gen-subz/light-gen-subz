import { render, screen, act, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { getVersion } from "@tauri-apps/api/app";
import { useSuiteMenu } from "../../suite/useSuiteMenu";
import { findSpec, type MenuSpec } from "../../suite/menu";
import { getT } from "../../i18n";
import { entry, lastMenu } from "../menuTestHelpers";
import { withSuite } from "./wrappers";

vi.mock("@tauri-apps/api/app", () => ({ getVersion: vi.fn() }));
vi.mock("@tauri-apps/plugin-updater", () => ({ check: vi.fn(() => Promise.resolve(null)) }));
vi.mock("@tauri-apps/plugin-process", () => ({ relaunch: vi.fn() }));

const openSettings = vi.fn();
const onLanguage = vi.fn();
const onCheckUpdates = vi.fn();

function Bar({ lang = "en" as "en" | "fr" }) {
  const file: MenuSpec[] = [
    {
      kind: "item",
      text: getT(lang)("menu.settings"),
      accelerator: "CmdOrCtrl+,",
      action: openSettings,
    },
  ];
  useSuiteMenu({ t: getT(lang), lang, appName: "Test", onLanguage, file, onCheckUpdates });
  return null;
}

async function mount(lang: "en" | "fr" = "en") {
  render(
    withSuite(<Bar lang={lang} />, {
      lang,
      changelogs: { en: "## [1.0.0] - 2026-01-01\n- first\n", fr: "" },
    }),
  );
  await waitFor(() => expect(lastMenu().length).toBeGreaterThan(0));
}

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  vi.mocked(getVersion).mockResolvedValue("1.0.0");
});

describe("the menu bar", () => {
  it("has File, View and Help (Edit only on macOS, where text fields need it)", async () => {
    await mount();

    expect(lastMenu().map((m) => m.text)).toEqual(["File", "View", "Help"]);
  });

  it("puts the app entries in File, before Quit", async () => {
    await mount();

    const file = lastMenu()[0].items;
    expect(file.map((e) => e.text ?? e.item)).toEqual(["Settings…", "Separator", "Quit"]);
    expect(entry("File", "Settings…").accelerator).toBe("CmdOrCtrl+,");
    entry("File", "Settings…").action?.();
    expect(openSettings).toHaveBeenCalled();
  });

  it("has no Edit menu outside macOS, where copy and paste work anyway", async () => {
    await mount();

    expect(lastMenu().some((m) => m.text === "Edit")).toBe(false);
  });

  it("has full screen and a theme choice in View", async () => {
    await mount();

    expect(entry("View", "Toggle Full Screen").accelerator).toBe("F11");
    const theme = entry("View", "Theme");
    expect(theme.items?.map((i) => [i.text, i.checked])).toEqual([
      ["Follow the system", true],
      ["Light", false],
      ["Dark", false],
    ]);
  });

  it("remembers the theme picked", async () => {
    await mount();

    await act(async () => entry("View", "Theme").items?.[2].action?.());

    expect(localStorage.getItem("suite:theme")).toBe("dark");
    await waitFor(() =>
      expect(entry("View", "Theme").items?.map((i) => i.checked)).toEqual([false, false, true]),
    );
  });

  it("has About, What is new, updates and language in Help", async () => {
    await mount();

    expect(
      lastMenu()[2]
        .items.map((e) => e.text)
        .filter(Boolean),
    ).toEqual(["About Test", "What's new", "Check for Updates…", "Language"]);
  });

  it("opens the About window with the version", async () => {
    await mount();

    await act(async () => entry("Help", "About Test").action?.());

    expect(await screen.findByRole("dialog", { name: "About Test" })).toBeInTheDocument();
    expect(screen.getByText("Version 1.0.0")).toBeInTheDocument();
  });

  it("opens the changelog", async () => {
    await mount();
    await screen.findByText("Test").catch(() => {});

    await act(async () => entry("Help", "What's new").action?.());

    expect(await screen.findByRole("dialog", { name: "What's new" })).toBeInTheDocument();
    expect(screen.getByText("first")).toBeInTheDocument();
  });

  it("checks for updates, and tells the app so it can add its own check", async () => {
    await mount();

    await act(async () => entry("Help", "Check for Updates…").action?.());

    expect(onCheckUpdates).toHaveBeenCalled();
    expect(await screen.findByText("You are up to date.")).toBeInTheDocument();
  });

  it("switches the language", async () => {
    await mount();

    entry("Help", "Language").items?.[1].action?.();

    expect(onLanguage).toHaveBeenCalledWith("fr");
  });

  it("is written in French when the app is", async () => {
    await mount("fr");

    expect(lastMenu().map((m) => m.text)).toEqual(["Fichier", "Affichage", "Aide"]);
    expect(entry("Aide", "À propos de Test")).toBeTruthy();
    expect(entry("Fichier", "Réglages…")).toBeTruthy();
  });
});

describe("findSpec", () => {
  it("finds an entry by the position the menu was built with", () => {
    const action = () => {};
    const menus = [
      {
        text: "A",
        items: [
          { kind: "separator" as const },
          {
            kind: "submenu" as const,
            text: "S",
            items: [{ kind: "item" as const, text: "x", action }],
          },
        ],
      },
    ];
    expect(findSpec(menus, "0.1.0")).toMatchObject({ text: "x" });
    expect(findSpec(menus, "0.5")).toBeUndefined();
  });
});
