import { render, screen, fireEvent, waitFor, act } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { getVersion } from "@tauri-apps/api/app";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { useSuite } from "../../suite/UpdaterProvider";
import { withSuite } from "./wrappers";

vi.mock("@tauri-apps/api/app", () => ({ getVersion: vi.fn() }));
vi.mock("@tauri-apps/plugin-updater", () => ({ check: vi.fn() }));
vi.mock("@tauri-apps/plugin-process", () => ({ relaunch: vi.fn() }));

const mockCheck = vi.mocked(check);
const mockRelaunch = vi.mocked(relaunch);

const NOTES = `## [0.3.0] - 2026-11-01\n### Fixed\n- a thing\n\n## [0.2.14] - 2026-10-01\n- old\n`;

function Trigger() {
  const { checkNow, openChangelog, version } = useSuite();
  return (
    <>
      <button onClick={() => void checkNow()}>check now</button>
      <button onClick={openChangelog}>open changelog</button>
      <span>version:{version}</span>
    </>
  );
}

const offer = (extra: Record<string, unknown> = {}) =>
  mockCheck.mockResolvedValue({
    version: "0.3.0",
    downloadAndInstall: vi.fn().mockResolvedValue(undefined),
    ...extra,
  } as never);

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  vi.mocked(getVersion).mockResolvedValue("0.2.14");
  mockCheck.mockResolvedValue(null);
});

afterEach(() => vi.useRealTimers());

async function ready() {
  await screen.findByText("version:0.2.14");
}

describe("update check", () => {
  it("says so when there is nothing new", async () => {
    render(withSuite(<Trigger />));
    await ready();

    fireEvent.click(screen.getByText("check now"));

    expect(await screen.findByText("You are up to date.")).toBeInTheDocument();
  });

  it("says so when the check fails, in the user language rather than the raw error", async () => {
    mockCheck.mockRejectedValue(new Error("update server unreachable"));
    render(withSuite(<Trigger />));
    await ready();

    fireEvent.click(screen.getByText("check now"));

    expect(await screen.findByText("The update failed — try again later.")).toBeInTheDocument();
    expect(screen.queryByText("update server unreachable")).toBeNull();
  });

  it("speaks French when the app does", async () => {
    render(withSuite(<Trigger />, { lang: "fr" }));
    await ready();

    fireEvent.click(screen.getByText("check now"));

    expect(await screen.findByText("Vous êtes à jour.")).toBeInTheDocument();
  });

  it("runs one check at a time", async () => {
    mockCheck.mockImplementation(() => new Promise(() => {}));
    render(withSuite(<Trigger />));
    await ready();

    fireEvent.click(screen.getByText("check now"));
    fireEvent.click(screen.getByText("check now"));

    await waitFor(() => expect(mockCheck).toHaveBeenCalledTimes(1));
  });

  it("checks by itself at start-up and stays quiet when there is nothing new", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const ui = withSuite(<Trigger />);
    // withSuite turns the start-up check off; render the real thing for this one.
    const { SuiteRoot } = await import("../../suite/SuiteRoot");
    const { getT } = await import("../../i18n");
    render(
      <SuiteRoot
        lang="en"
        t={getT("en")}
        appId="t"
        appName="Test"
        changelogs={{ en: "", fr: "" }}
        fetchNotes={async () => ""}
      >
        <Trigger />
      </SuiteRoot>,
    );
    void ui;
    await act(async () => {
      await vi.advanceTimersByTimeAsync(3500);
    });

    expect(mockCheck).toHaveBeenCalledTimes(1);
    expect(screen.queryByText("You are up to date.")).toBeNull();
  });
});

describe("update banner", () => {
  it("offers the new version with what it brings, in the user language", async () => {
    offer();
    const fetchNotes = vi.fn().mockResolvedValue(NOTES);
    render(withSuite(<Trigger />, { fetchNotes }));
    await ready();

    fireEvent.click(screen.getByText("check now"));

    expect(await screen.findByText("Version 0.3.0 is available.")).toBeInTheDocument();
    expect(fetchNotes).toHaveBeenCalledWith("en", "0.3.0");
    fireEvent.click(screen.getByRole("button", { name: "What's new" }));
    expect(screen.getByText(/• a thing/)).toBeInTheDocument();
    // Only what is newer than the installed version.
    expect(screen.queryByText(/old/)).toBeNull();
  });

  it("still offers the update when the notes cannot be read", async () => {
    offer();
    render(withSuite(<Trigger />, { fetchNotes: () => Promise.reject(new Error("offline")) }));
    await ready();

    fireEvent.click(screen.getByText("check now"));

    expect(await screen.findByText("Version 0.3.0 is available.")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "What's new" })).toBeNull();
  });

  it('goes away on "Later"', async () => {
    offer();
    render(withSuite(<Trigger />));
    await ready();
    fireEvent.click(screen.getByText("check now"));
    await screen.findByText("Version 0.3.0 is available.");

    fireEvent.click(screen.getByRole("button", { name: "Later" }));

    expect(screen.queryByText("Version 0.3.0 is available.")).toBeNull();
  });

  it("installs, shows the progress, and restarts", async () => {
    const downloadAndInstall = vi.fn(async (onEvent: (e: Record<string, unknown>) => void) => {
      onEvent({ event: "Started", data: { contentLength: 200 } });
      onEvent({ event: "Progress", data: { chunkLength: 100 } });
      onEvent({ event: "Finished", data: {} });
    });
    offer({ downloadAndInstall });
    render(withSuite(<Trigger />));
    await ready();
    fireEvent.click(screen.getByText("check now"));
    expect(downloadAndInstall).not.toHaveBeenCalled();

    fireEvent.click(await screen.findByRole("button", { name: "Update now" }));

    await waitFor(() => expect(downloadAndInstall).toHaveBeenCalled());
    await waitFor(() => expect(mockRelaunch).toHaveBeenCalled());
  });

  it("copes with a download that reports no total size", async () => {
    offer({
      downloadAndInstall: vi.fn(async (onEvent: (e: Record<string, unknown>) => void) => {
        onEvent({ event: "Started", data: {} });
        onEvent({ event: "Progress", data: { chunkLength: 50 } });
      }),
    });
    render(withSuite(<Trigger />));
    await ready();
    fireEvent.click(screen.getByText("check now"));

    fireEvent.click(await screen.findByRole("button", { name: "Update now" }));

    await waitFor(() => expect(mockRelaunch).toHaveBeenCalled());
  });

  it("reports an install that fails after the download", async () => {
    offer({ downloadAndInstall: vi.fn().mockRejectedValue(new Error("signature mismatch")) });
    render(withSuite(<Trigger />));
    await ready();
    fireEvent.click(screen.getByText("check now"));

    fireEvent.click(await screen.findByRole("button", { name: "Update now" }));

    expect(await screen.findByText("The update failed — try again later.")).toBeInTheDocument();
    expect(mockRelaunch).not.toHaveBeenCalled();
  });
});

describe("What's new window", () => {
  const changelogs = {
    en: "## [0.3.0] - 2026-11-01\n### Fixed\n- **future** thing\n\n## [0.2.14] - 2026-10-01\n### Added\n- a `shipped` thing\n",
    fr: "## [0.2.14] - 2026-10-01\n### Ajouté\n- une chose livrée\n",
  };

  it("lists what shipped up to the installed version, in the user language", async () => {
    const { rerender } = render(withSuite(<Trigger />, { changelogs }));
    await ready();

    fireEvent.click(screen.getByText("open changelog"));

    expect(await screen.findByText("v0.2.14")).toBeInTheDocument();
    expect(screen.getByText("shipped")).toBeInTheDocument();
    expect(screen.queryByText(/future/)).toBeNull();

    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("dialog")).toBeNull();

    rerender(withSuite(<Trigger />, { changelogs, lang: "fr" }));
    fireEvent.click(screen.getByText("open changelog"));
    expect(await screen.findByText("une chose livrée")).toBeInTheDocument();
  });

  it("opens by itself once after an update, not on a first install", async () => {
    const first = render(withSuite(<Trigger />, { changelogs }));
    await ready();
    expect(screen.queryByRole("dialog")).toBeNull();
    first.unmount();

    // The next launch is a newer version.
    localStorage.setItem("test:suite:last-seen-version", "0.2.0");
    render(withSuite(<Trigger />, { changelogs }));

    expect(await screen.findByRole("dialog", { name: "What's new" })).toBeInTheDocument();
  });
});
