import "@testing-library/jest-dom/vitest";
import { vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/api/app", () => ({
  getVersion: vi.fn(() => Promise.resolve("0.2.3")),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(() => {})),
  emit: vi.fn(),
}));

vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: vi.fn(() => ({
    onDragDropEvent: vi.fn(() => Promise.resolve(() => {})),
  })),
}));

vi.mock("@tauri-apps/plugin-updater", () => ({
  check: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-process", () => ({
  relaunch: vi.fn(),
}));

// The native menu bar and the window: recorded instead of built, so tests can read the menu
// the app installs and click its entries.
vi.mock("@tauri-apps/api/menu", () => {
  const make = (kind: string) => ({ new: vi.fn(async (options: object) => ({ kind, ...options })) });
  return {
    Menu: { new: vi.fn(async (options: object) => ({ ...options, setAsAppMenu: vi.fn() })) },
    Submenu: make("submenu"),
    MenuItem: make("item"),
    CheckMenuItem: make("check"),
    PredefinedMenuItem: make("predefined"),
  };
});

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    setTheme: vi.fn(async () => {}),
    isFullscreen: vi.fn(async () => false),
    setFullscreen: vi.fn(async () => {}),
  }),
}));
