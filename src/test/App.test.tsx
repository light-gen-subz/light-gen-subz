import { render, screen, fireEvent, waitFor, act, within } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import App from "../App";
import { entry, lastMenu } from "./menuTestHelpers";

const mockInvoke = vi.mocked(invoke);
const mockListen = vi.mocked(listen);
const mockWebview = vi.mocked(getCurrentWebview);
const mockCheck = vi.mocked(check);
const mockRelaunch = vi.mocked(relaunch);

const SRT = ["1", "00:00:00,000 --> 00:00:02,000", "Hello there"].join("\n");
const SRT_TWO = [SRT, ["2", "00:00:02,000 --> 00:00:04,000", "General Kenobi"].join("\n")].join(
  "\n\n",
);

const settings = {
  stt_engine: "local",
  translation_engine: "local",
  azure_translator_region: "",
  language: "en",
  whisper_model: "small",
  stt_language: "auto",
};

const languages = [
  { code: "en", flores_code: "eng_Latn", name: "English" },
  { code: "fr", flores_code: "fra_Latn", name: "French" },
];

/** Per-command responses layered on top of the defaults. */
let responses: Record<string, unknown>;
/** The handler the app registered for pipeline-progress events. */
let onProgress: ((e: { payload: unknown }) => void) | undefined;

function setResponse(command: string, value: unknown) {
  responses[command] = value;
}

async function renderAndGetContainer() {
  const view = render(<App />);
  await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("get_settings"));
  await act(async () => {});
  return view;
}

async function renderApp() {
  render(<App />);
  // settle the two settings/languages effects
  await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("get_settings"));
  await act(async () => {});
}

/** Exports through the Export window: header button, then format (and subtitles), then confirm. */
function exportAs(
  format: "srt" | "vtt" | "ass",
  target: "source" | "translation" | "both" = "source",
) {
  fireEvent.click(screen.getByRole("button", { name: "Export" }));
  const dialog = screen.getByRole("dialog", { name: "Export subtitles" });
  if (target !== "source") fireEvent.click(within(dialog).getByLabelText("Translation"));
  if (target === "translation") fireEvent.click(within(dialog).getByLabelText("Original"));
  fireEvent.click(within(dialog).getByLabelText(new RegExp(`\\.${format}\\)`)));
  fireEvent.click(within(dialog).getByRole("button", { name: "Export" }));
}

/** Picks a file and waits for the chip to appear. */
async function pickFile(path = "/home/me/videos/clip.mp4") {
  setResponse("pick_file", path);
  fireEvent.click(screen.getByRole("button", { name: /browse files/i }));
  await screen.findAllByText("clip.mp4");
}

/** Same as `pickFile`, for an interface in French. */
async function pickFileFr() {
  setResponse("pick_file", "/home/me/videos/clip.mp4");
  fireEvent.click(screen.getByRole("button", { name: /parcourir les fichiers/i }));
  await screen.findAllByText("clip.mp4");
}

/** Runs a successful generation and waits for the result panel. */
async function generate(output: Record<string, unknown> = {}) {
  setResponse("run_pipeline", {
    srt_path: "/home/me/videos/clip.srt",
    srt_content: SRT,
    language: "en",
    ...output,
  });
  fireEvent.click(screen.getByRole("button", { name: "Generate subtitles" }));
  await screen.findByText("clip.srt");
}

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  onProgress = undefined;
  responses = {
    get_settings: { ...settings },
    list_languages: languages,
    has_api_key: false,
    pick_file: null,
    set_settings: undefined,
    set_api_key: undefined,
    save_subtitle_as: ["/home/me/final.srt"],
    list_audio_tracks: [],
    delete_api_key: undefined,
    cancel_pipeline: undefined,
  };
  mockInvoke.mockImplementation((command: string) => {
    if (command in responses) return Promise.resolve(responses[command]);
    return Promise.reject(new Error(`unstubbed command: ${command}`));
  });
  mockListen.mockImplementation(((event: string, handler: unknown) => {
    if (event === "pipeline-progress") {
      onProgress = handler as (e: { payload: unknown }) => void;
    }
    return Promise.resolve(() => {});
  }) as typeof listen);
});

describe("App — initial state", () => {
  it("offers the file picker, with no header until a file is open", async () => {
    await renderApp();

    expect(screen.getByRole("button", { name: /browse files/i })).toBeInTheDocument();
    expect(screen.queryByRole("status")).toBeNull();
    expect(document.querySelector("header")).toBeNull();
  });

  it("shows the file name and its details in the top bar once a file is open", async () => {
    await renderApp();
    await pickFile();

    const bar = document.querySelector("header") as HTMLElement;
    expect(within(bar).getByText("clip.mp4")).toBeInTheDocument();
    expect(within(bar).getByText("MP4")).toBeInTheDocument();

    await generate();

    expect(within(bar).getByText(/MP4 · EN · \d+ lines?/)).toBeInTheDocument();
  });

  it("loads settings and languages on mount", async () => {
    await renderApp();

    expect(mockInvoke).toHaveBeenCalledWith("get_settings");
    expect(mockInvoke).toHaveBeenCalledWith("list_languages");
  });
});

describe("App — choosing a file", () => {
  it("shows the file name and a generate button once a file is picked", async () => {
    await renderApp();
    await pickFile();

    expect(screen.getAllByText("clip.mp4").length).toBe(2);
    expect(screen.getByRole("button", { name: "Generate subtitles" })).toBeInTheDocument();
  });

  it("keeps the dropzone when the picker is dismissed", async () => {
    await renderApp();
    setResponse("pick_file", null);

    fireEvent.click(screen.getByRole("button", { name: /browse files/i }));
    await act(async () => {});

    expect(screen.getByRole("button", { name: /browse files/i })).toBeInTheDocument();
  });

  it("lists the files opened lately on the start screen, newest first, and reopens one", async () => {
    await renderApp();
    setResponse("pick_file", "/home/me/videos/one.mp4");
    fireEvent.click(screen.getByRole("button", { name: /browse files/i }));
    await screen.findAllByText("one.mp4");
    setResponse("pick_file", "/home/me/videos/two.mkv");
    fireEvent.click(screen.getByRole("button", { name: "Change" }));
    await screen.findAllByText("two.mkv");
    expect(JSON.parse(localStorage.getItem("light-gen-subz:recent") ?? "[]")).toEqual([
      "/home/me/videos/two.mkv",
      "/home/me/videos/one.mp4",
    ]);
  });

  it("shows the recent files, removes one, clears the list", async () => {
    localStorage.setItem(
      "light-gen-subz:recent",
      JSON.stringify(["/home/me/a.mp4", "/home/me/b.wav"]),
    );
    await renderApp();

    expect(screen.getByText("a.mp4")).toBeInTheDocument();
    expect(screen.getByText("WAV")).toBeInTheDocument();
    fireEvent.click(screen.getAllByRole("button", { name: "Remove from the list" })[0]);
    expect(screen.queryByText("a.mp4")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Clear" }));
    expect(screen.queryByText("b.wav")).toBeNull();
    expect(localStorage.getItem("light-gen-subz:recent")).toBe("[]");
  });

  it("opens a recent file", async () => {
    localStorage.setItem("light-gen-subz:recent", JSON.stringify(["/home/me/a.mp4"]));
    await renderApp();

    fireEvent.click(screen.getByRole("button", { name: /a\.mp4/ }));

    expect(await screen.findByRole("button", { name: "Generate subtitles" })).toBeInTheDocument();
  });

  it("lets the file be swapped through the Change button", async () => {
    await renderApp();
    await pickFile();

    setResponse("pick_file", "/home/me/other.mov");
    fireEvent.click(screen.getByRole("button", { name: "Change" }));

    expect((await screen.findAllByText("other.mov")).length).toBeGreaterThan(0);
  });
});

describe("App — generating subtitles", () => {
  it("shows the transcript, its language and line count on success", async () => {
    await renderApp();
    await pickFile();
    await generate({ srt_content: SRT_TWO });

    expect(screen.getByText("EN")).toBeInTheDocument();
    expect(screen.getByText("2 lines")).toBeInTheDocument();
    expect(screen.getByText("Hello there")).toBeInTheDocument();
    expect(screen.getByText("General Kenobi")).toBeInTheDocument();
  });

  it("uses the singular wording for a single cue", async () => {
    await renderApp();
    await pickFile();
    await generate();

    expect(screen.getByText("1 line")).toBeInTheDocument();
  });

  it("surfaces a pipeline failure", async () => {
    await renderApp();
    await pickFile();
    mockInvoke.mockImplementation((command: string) =>
      command === "run_pipeline"
        ? Promise.reject(new Error("ffmpeg missing"))
        : Promise.resolve(responses[command]),
    );

    fireEvent.click(screen.getByRole("button", { name: "Generate subtitles" }));

    expect(await screen.findByText(/Generation failed/)).toBeInTheDocument();
    expect(screen.getByText(/ffmpeg missing/)).toBeInTheDocument();
  });

  it("passes the chosen file to the backend", async () => {
    await renderApp();
    await pickFile();
    await generate();

    expect(mockInvoke).toHaveBeenCalledWith("run_pipeline", {
      inputPath: "/home/me/videos/clip.mp4",
      audioTrack: null,
    });
  });
});

describe("App — progress reporting", () => {
  it("walks the stepper as stages are reported", async () => {
    await renderApp();
    await pickFile();

    let resolvePipeline: (v: unknown) => void = () => {};
    mockInvoke.mockImplementation((command: string) =>
      command === "run_pipeline"
        ? new Promise((res) => {
            resolvePipeline = res;
          })
        : Promise.resolve(responses[command]),
    );
    fireEvent.click(screen.getByRole("button", { name: "Generate subtitles" }));
    await screen.findByRole("button", { name: "Generating…" });

    act(() => onProgress?.({ payload: { stage: "transcribe", fraction: 0.5 } }));

    const active = document.querySelector(".step--active .step-label");
    expect(active?.textContent).toBe("Transcript");
    expect(document.querySelectorAll(".step--done")).toHaveLength(2);

    await act(async () => {
      resolvePipeline({ srt_path: "/p.srt", srt_content: SRT, language: "en" });
    });
  });

  it("hides the model-download step for cloud transcription engines", async () => {
    setResponse("get_settings", { ...settings, stt_engine: "groq" });
    await renderApp();
    await pickFile();

    let resolvePipeline: (v: unknown) => void = () => {};
    mockInvoke.mockImplementation((command: string) =>
      command === "run_pipeline"
        ? new Promise((res) => {
            resolvePipeline = res;
          })
        : Promise.resolve(responses[command]),
    );
    fireEvent.click(screen.getByRole("button", { name: "Generate subtitles" }));
    await screen.findByRole("button", { name: "Generating…" });

    const labels = [...document.querySelectorAll(".step-label")].map((n) => n.textContent);
    expect(labels).toEqual(["Audio", "Transcript", "Subtitles"]);

    await act(async () => {
      resolvePipeline({ srt_path: "/p.srt", srt_content: SRT, language: "en" });
    });
  });
});

describe("App — saving", () => {
  it("asks the backend to save the transcript, which opens the dialog itself", async () => {
    await renderApp();
    await pickFile();
    await generate();

    exportAs("srt");

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("save_subtitle_as", {
        defaultPath: "/home/me/videos/clip.srt",
        filterName: "SRT subtitles",
        format: "srt",
        srtContent: SRT,
        extras: [],
      }),
    );
  });

  it("can export the transcript as WebVTT", async () => {
    await renderApp();
    await pickFile();
    await generate();

    exportAs("vtt");

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith(
        "save_subtitle_as",
        expect.objectContaining({ format: "vtt", filterName: "WebVTT subtitles" }),
      ),
    );
  });

  it("no longer lets the webview write an arbitrary path", async () => {
    await renderApp();
    await pickFile();
    await generate();

    exportAs("srt");
    await act(async () => {});

    expect(mockInvoke).not.toHaveBeenCalledWith("save_subtitle", expect.anything());
  });

  it("writes nothing by itself: the result is only offered under a name, exported on demand", async () => {
    await renderApp();
    await pickFile();
    await generate();

    expect(mockInvoke).not.toHaveBeenCalledWith("save_subtitle_as", expect.anything());
    expect(screen.getByText("Hello there")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).toBeNull();
  });
});

describe("App — settings panel", () => {
  it("is hidden until the settings button is pressed", async () => {
    await renderApp();
    expect(screen.queryByText("Transcription engine")).not.toBeInTheDocument();

    fireEvent.keyDown(window, { key: ",", ctrlKey: true });

    expect(screen.getByText("Transcription engine")).toBeInTheDocument();
  });

  it("persists a change of transcription engine", async () => {
    await renderApp();
    fireEvent.keyDown(window, { key: ",", ctrlKey: true });

    fireEvent.change(screen.getByDisplayValue("Local (offline, whisper.cpp)"), {
      target: { value: "groq" },
    });

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("set_settings", {
        settings: { ...settings, stt_engine: "groq" },
      }),
    );
  });

  it("persists a change of translation engine", async () => {
    await renderApp();
    fireEvent.keyDown(window, { key: ",", ctrlKey: true });

    fireEvent.change(screen.getByDisplayValue("Local (offline, NLLB-200)"), {
      target: { value: "deep_l" },
    });

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("set_settings", {
        settings: { ...settings, translation_engine: "deep_l" },
      }),
    );
  });

  it("asks for a region only for Azure", async () => {
    setResponse("get_settings", { ...settings, translation_engine: "azure" });
    await renderApp();

    fireEvent.keyDown(window, { key: ",", ctrlKey: true });

    expect(screen.getByText("Azure region")).toBeInTheDocument();
  });

  it("persists the Azure region as it is typed", async () => {
    setResponse("get_settings", { ...settings, translation_engine: "azure" });
    await renderApp();
    fireEvent.keyDown(window, { key: ",", ctrlKey: true });

    fireEvent.change(screen.getByPlaceholderText("westeurope"), {
      target: { value: "westeurope" },
    });

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("set_settings", {
        settings: {
          ...settings,
          translation_engine: "azure",
          azure_translator_region: "westeurope",
        },
      }),
    );
  });
});

describe("App — language", () => {
  async function openSettings(lang = "en") {
    setResponse("get_settings", { ...settings, language: lang });
    await renderApp();
    fireEvent.keyDown(window, { key: ",", ctrlKey: true });
  }

  it("starts in English", async () => {
    await openSettings();

    expect(screen.getByText("Transcription engine")).toBeInTheDocument();
  });

  it("opens in French when that is what was saved", async () => {
    await openSettings("fr");

    expect(screen.getByText("Moteur de transcription")).toBeInTheDocument();
    expect(screen.queryByText("Transcription engine")).not.toBeInTheDocument();
  });

  it("persists the chosen language", async () => {
    await openSettings();

    await act(async () => entry("Help", "Language").items?.[1].action?.());

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("set_settings", {
        settings: { ...settings, language: "fr" },
      }),
    );
  });

  it("switches the interface without waiting for a restart", async () => {
    await openSettings();

    await act(async () => entry("Help", "Language").items?.[1].action?.());

    expect(await screen.findByText("Moteur de transcription")).toBeInTheDocument();
  });

  it("translates the body, not just the settings", async () => {
    setResponse("get_settings", { ...settings, language: "fr" });
    await renderApp();

    expect(screen.getByRole("button", { name: /parcourir les fichiers/i })).toBeInTheDocument();
  });

  it("tells the page which language it is in, for the spell checker and screen readers", async () => {
    setResponse("get_settings", { ...settings, language: "fr" });
    await renderApp();

    expect(document.documentElement.lang).toBe("fr");
  });
});

describe("App — version", () => {
  it("shows the running version in the About window", async () => {
    await renderApp();

    await act(async () => entry("Help", "About LightGenSubz").action?.());

    expect(await screen.findByText("Version 0.2.3")).toBeInTheDocument();
  });

  it("does not leave the app unusable when the version cannot be read", async () => {
    vi.mocked(getVersion).mockRejectedValueOnce(new Error("no ipc"));
    await renderApp();

    fireEvent.keyDown(window, { key: ",", ctrlKey: true });

    expect(screen.getByText("Transcription engine")).toBeInTheDocument();
  });
});

describe("App — menu bar", () => {
  it("has the File, Edit, View and Help menus of the suite", async () => {
    await renderApp();

    expect(lastMenu().map((m) => m.text)).toEqual(["File", "View", "Help"]);
    expect(lastMenu()[0].items.map((e) => e.text ?? e.item)).toEqual([
      "Choose a file…",
      "Open recent",
      "Separator",
      "Settings…",
      "Separator",
      "Quit",
    ]);
  });

  it("has no Settings button in the window any more", async () => {
    await renderApp();

    expect(screen.queryByRole("button", { name: "Settings" })).toBeNull();
  });

  it("opens and closes the settings from File", async () => {
    await renderApp();

    await act(async () => entry("File", "Settings…").action?.());
    expect(screen.getByText("Transcription engine")).toBeInTheDocument();
    await act(async () => entry("File", "Settings…").action?.());
    expect(screen.queryByText("Transcription engine")).toBeNull();
  });

  it("shows the settings in a dialog, closed by its button, Escape or the backdrop", async () => {
    await renderApp();

    await act(async () => entry("File", "Settings…").action?.());
    const dialog = screen.getByRole("dialog", { name: "Settings" });
    expect(dialog).toContainElement(screen.getByText("Transcription engine"));

    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(screen.queryByRole("dialog")).toBeNull();

    await act(async () => entry("File", "Settings…").action?.());
    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("shows the settings in a sheet on the right, closed by its button, Escape or the backdrop", async () => {
    const { container } = await renderAndGetContainer();
    await act(async () => entry("File", "Settings…").action?.());
    const sheet = screen.getByRole("dialog", { name: "Settings" });
    expect(sheet).toContainElement(screen.getByText("Transcription engine"));

    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(screen.queryByRole("dialog")).toBeNull();

    await act(async () => entry("File", "Settings…").action?.());
    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("dialog")).toBeNull();

    await act(async () => entry("File", "Settings…").action?.());
    fireEvent.click(container.querySelector(".sheet-overlay") as Element);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("picks a file from File, Ctrl+O included", async () => {
    await renderApp();
    setResponse("pick_file", "/home/me/videos/clip.mp4");

    await act(async () => entry("File", "Choose a file…").action?.());

    expect((await screen.findAllByText("clip.mp4")).length).toBeGreaterThan(0);
  });

  it("lists the recent files in File > Open recent, and reopens one", async () => {
    localStorage.setItem("light-gen-subz:recent", JSON.stringify(["/home/me/a.mp4"]));
    await renderApp();

    const recent = entry("File", "Open recent");
    expect(recent.items?.map((i) => i.text ?? i.item)).toEqual(["a.mp4", "Separator", "Clear"]);
    await act(async () => recent.items?.[0].action?.());

    expect(await screen.findByRole("button", { name: "Generate subtitles" })).toBeInTheDocument();
  });

  it("is written in French when the interface is", async () => {
    setResponse("get_settings", { ...settings, language: "fr" });
    await renderApp();

    expect(lastMenu().map((m) => m.text)).toEqual(["Fichier", "Affichage", "Aide"]);
  });
});

describe("App — export and result actions", () => {
  it("disables Export and Regenerate until subtitles exist", async () => {
    await renderApp();
    await pickFile();

    expect(screen.getByRole("button", { name: "Export" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Regenerate" })).toBeDisabled();

    await generate();

    expect(screen.getByRole("button", { name: "Export" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Regenerate" })).toBeEnabled();
  });

  it("has no text buttons left in the result block: only an Edit icon with a tooltip", async () => {
    await renderApp();
    await pickFile();
    await generate();

    expect(screen.queryByRole("button", { name: /Save as|Export \.|Export as/ })).toBeNull();
    const edit = screen.getByRole("button", { name: "Edit" });
    expect(edit).toHaveAttribute("title", "Edit");
    expect(edit.textContent).toBe("");
    expect(screen.getByRole("button", { name: "Regenerate" })).toHaveAttribute(
      "title",
      "Regenerate",
    );
  });

  it("offers the three formats in the Export window, and closes without saving", async () => {
    await renderApp();
    await pickFile();
    await generate();

    fireEvent.click(screen.getByRole("button", { name: "Export" }));
    const dialog = screen.getByRole("dialog", { name: "Export subtitles" });

    expect(
      within(dialog)
        .getAllByRole("radio")
        .map((r) => r.closest("label")?.textContent),
    ).toEqual(["SubRip (.srt)", "WebVTT (.vtt)", "Advanced SubStation (.ass)"]);
    expect(within(dialog).queryByLabelText("Translation")).toBeNull();
    fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(mockInvoke).not.toHaveBeenCalledWith("save_subtitle_as", expect.anything());
  });

  it("exports the original and the translation at once, with a single save dialog", async () => {
    setResponse("get_settings", { ...settings, translation_engine: "deep_l" });
    await renderApp();
    await pickFile();
    await generate();
    setResponse("translate_subtitles", {
      srt_path: "/home/me/videos/clip.fr.srt",
      srt_content: SRT,
      language: "fr",
    });
    fireEvent.click(screen.getByRole("button", { name: "Translate" }));
    await screen.findByText("clip.fr.srt");
    setResponse("save_subtitle_as", ["/home/me/out.srt", "/home/me/out.fr.srt"]);

    // With nothing ticked there is nothing to export.
    fireEvent.click(screen.getByRole("button", { name: "Export" }));
    const dialog = screen.getByRole("dialog", { name: "Export subtitles" });
    fireEvent.click(within(dialog).getByLabelText("Original"));
    expect(within(dialog).getByRole("button", { name: "Export" })).toBeDisabled();
    fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));

    exportAs("srt", "both");

    // One dialog, one call: the translation travels as an extra file saved next to the original.
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("save_subtitle_as", {
        defaultPath: "/home/me/videos/clip.srt",
        filterName: "SRT subtitles",
        format: "srt",
        srtContent: SRT,
        extras: [{ srt_path: "/home/me/videos/clip.fr.srt", srt_content: SRT }],
      }),
    );
    expect(mockInvoke.mock.calls.filter(([c]) => c === "save_subtitle_as")).toHaveLength(1);
    expect(await screen.findByText(/out\.srt, \/home\/me\/out\.fr\.srt/)).toBeInTheDocument();
  });

  it("has no Export button on the start screen", async () => {
    await renderApp();

    expect(screen.queryByRole("button", { name: "Export" })).toBeNull();
  });
});

describe("App — API key field", () => {
  it("appears for a cloud engine and reports a stored key", async () => {
    setResponse("get_settings", { ...settings, stt_engine: "groq" });
    setResponse("has_api_key", true);
    await renderApp();

    fireEvent.keyDown(window, { key: ",", ctrlKey: true });

    expect(await screen.findByText(/API key \(saved\)/)).toBeInTheDocument();
  });

  it("stores a trimmed key and clears the input", async () => {
    setResponse("get_settings", { ...settings, stt_engine: "groq" });
    await renderApp();
    fireEvent.keyDown(window, { key: ",", ctrlKey: true });

    const input = screen.getByPlaceholderText("gsk_...");
    fireEvent.change(input, { target: { value: "  gsk_secret  " } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("set_api_key", {
        keyName: "groq_api_key",
        value: "gsk_secret",
      }),
    );
    expect((input as HTMLInputElement).value).toBe("");
    expect(await screen.findByText(/API key \(saved\)/)).toBeInTheDocument();
  });

  it("ignores a blank key", async () => {
    setResponse("get_settings", { ...settings, stt_engine: "groq" });
    await renderApp();
    fireEvent.keyDown(window, { key: ",", ctrlKey: true });

    fireEvent.change(screen.getByPlaceholderText("gsk_..."), { target: { value: "   " } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await act(async () => {});

    expect(mockInvoke).not.toHaveBeenCalledWith("set_api_key", expect.anything());
  });
});

describe("App — updates", () => {
  it("reports being up to date", async () => {
    mockCheck.mockResolvedValue(null);
    await renderApp();
    fireEvent.keyDown(window, { key: ",", ctrlKey: true });

    await act(async () => entry("Help", "Check for Updates…").action?.());

    expect(await screen.findByText("You are up to date.")).toBeInTheDocument();
  });

  it("offers an available update in a banner, then installs it and relaunches", async () => {
    const downloadAndInstall = vi.fn().mockResolvedValue(undefined);
    mockCheck.mockResolvedValue({ version: "9.9.9", downloadAndInstall } as never);
    await renderApp();
    fireEvent.keyDown(window, { key: ",", ctrlKey: true });

    await act(async () => entry("Help", "Check for Updates…").action?.());
    expect(await screen.findByText("Version 9.9.9 is available.")).toBeInTheDocument();
    expect(downloadAndInstall).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Update now" }));

    await waitFor(() => expect(downloadAndInstall).toHaveBeenCalled());
    await waitFor(() => expect(mockRelaunch).toHaveBeenCalled());
  });

  it("tells the user the check failed, without the raw error", async () => {
    mockCheck.mockRejectedValue(new Error("network down"));
    await renderApp();
    fireEvent.keyDown(window, { key: ",", ctrlKey: true });

    await act(async () => entry("Help", "Check for Updates…").action?.());

    expect(await screen.findByText("The update failed — try again later.")).toBeInTheDocument();
    expect(screen.queryByText(/network down/)).toBeNull();
  });

  it("shows the changelog from the settings", async () => {
    await renderApp();
    fireEvent.keyDown(window, { key: ",", ctrlKey: true });

    await act(async () => entry("Help", "What's new").action?.());

    expect(await screen.findByRole("dialog", { name: "What's new" })).toBeInTheDocument();
  });
});

describe("App — translation block", () => {
  it("is offered after each generation, whatever the engine, and nothing runs by itself", async () => {
    await renderApp();
    await pickFile();

    await generate();

    expect(screen.getByRole("button", { name: "Translate" })).toBeInTheDocument();
    expect(mockInvoke).not.toHaveBeenCalledWith("translate_subtitles", expect.anything());
  });

  it("has no per-run translation choice next to the file", async () => {
    await renderApp();
    await pickFile();

    expect(screen.queryByText("Subtitles and translation")).toBeNull();
    expect(screen.queryByRole("radio", { name: /translation/i })).toBeNull();
  });

  it("has no Off entry among the translation engines of the settings", async () => {
    await renderApp();
    fireEvent.keyDown(window, { key: ",", ctrlKey: true });

    const select = screen.getByDisplayValue("Local (offline, NLLB-200)");
    expect(within(select).queryByText("Off")).toBeNull();
    expect(within(select).getAllByRole("option")).toHaveLength(5);
  });
});

describe("App — translation", () => {
  const withTranslation = { ...settings, translation_engine: "local" };

  it("offers translation after a generation with the engine configured", async () => {
    setResponse("get_settings", withTranslation);
    await renderApp();
    await pickFile();
    await generate();

    expect(screen.getByRole("button", { name: "Translate" })).toBeInTheDocument();
  });

  it("preselects the detected source language", async () => {
    setResponse("get_settings", withTranslation);
    await renderApp();
    await pickFile();
    await generate({ language: "FR" });

    expect(screen.getByDisplayValue("French")).toBeInTheDocument();
  });

  it("translates and lists the translated cues", async () => {
    setResponse("get_settings", withTranslation);
    setResponse("translate_subtitles", {
      srt_path: "/home/me/videos/clip.fr.srt",
      srt_content: ["1", "00:00:00,000 --> 00:00:02,000", "Bonjour"].join("\n"),
      language: "fr",
    });
    await renderApp();
    await pickFile();
    await generate();

    fireEvent.click(screen.getByRole("button", { name: "Translate" }));

    expect(await screen.findByText("Bonjour")).toBeInTheDocument();
    expect(screen.getByText("clip.fr.srt")).toBeInTheDocument();
  });

  it("sends null as the source language when none was detected", async () => {
    setResponse("get_settings", withTranslation);
    setResponse("translate_subtitles", { srt_path: "/p.srt", srt_content: SRT, language: "fr" });
    await renderApp();
    await pickFile();
    await generate({ language: "" });

    fireEvent.click(screen.getByRole("button", { name: "Translate" }));

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("translate_subtitles", {
        srtPath: "/home/me/videos/clip.srt",
        srtContent: SRT,
        sourceLang: null,
        targetLang: "en",
      }),
    );
  });

  it("gives a cloud translator the detected language instead of letting it guess line by line", async () => {
    setResponse("get_settings", { ...settings, translation_engine: "deep_l" });
    setResponse("translate_subtitles", { srt_path: "/p.srt", srt_content: SRT, language: "en" });
    await renderApp();
    await pickFile();
    await generate({ language: "fr" });

    fireEvent.click(screen.getByRole("button", { name: "Translate" }));

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith(
        "translate_subtitles",
        expect.objectContaining({ sourceLang: "fr", targetLang: "en" }),
      ),
    );
  });

  it("labels the language of the translation shown, not the one picked afterwards", async () => {
    setResponse("get_settings", withTranslation);
    setResponse("translate_subtitles", { srt_path: "/p.fr.srt", srt_content: SRT, language: "fr" });
    await renderApp();
    await pickFile();
    await generate();
    fireEvent.click(screen.getByRole("button", { name: "Translate" }));
    await screen.findByText("p.fr.srt");

    const [, target] = screen.getAllByRole("combobox");
    fireEvent.change(target, { target: { value: "en" } });

    expect(screen.getByText("FR", { selector: ".pill" })).toBeInTheDocument();
  });

  it("closes the editor of the previous translation when a new one starts", async () => {
    setResponse("get_settings", withTranslation);
    setResponse("translate_subtitles", { srt_path: "/p.fr.srt", srt_content: SRT, language: "fr" });
    await renderApp();
    await pickFile();
    await generate();
    fireEvent.click(screen.getByRole("button", { name: "Translate" }));
    await screen.findByText("p.fr.srt");
    fireEvent.click(screen.getAllByRole("button", { name: "Edit" })[1]);

    fireEvent.click(screen.getByRole("button", { name: "Translate" }));

    await waitFor(() =>
      expect(screen.queryByRole("button", { name: "Apply changes" })).not.toBeInTheDocument(),
    );
  });

  it("cannot translate while the source is being edited", async () => {
    setResponse("get_settings", withTranslation);
    await renderApp();
    await pickFile();
    await generate();

    fireEvent.click(screen.getAllByRole("button", { name: "Edit" })[0]);

    expect(screen.getByRole("button", { name: "Translate" })).toBeDisabled();
  });

  it("reports a translation failure inline", async () => {
    setResponse("get_settings", withTranslation);
    await renderApp();
    await pickFile();
    await generate();
    mockInvoke.mockImplementation((command: string) =>
      command === "translate_subtitles"
        ? Promise.reject(new Error("model missing"))
        : Promise.resolve(responses[command]),
    );

    fireEvent.click(screen.getByRole("button", { name: "Translate" }));

    expect(await screen.findByText(/Translation failed: .*model missing/)).toBeInTheDocument();
  });

  it("saves the translated file to the chosen destination", async () => {
    setResponse("get_settings", withTranslation);
    setResponse("translate_subtitles", {
      srt_path: "/p.fr.srt",
      srt_content: "translated",
      language: "fr",
    });
    await renderApp();
    await pickFile();
    await generate();
    fireEvent.click(screen.getByRole("button", { name: "Translate" }));
    await screen.findByText("p.fr.srt");

    exportAs("srt", "translation");

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("save_subtitle_as", {
        defaultPath: "/p.fr.srt",
        filterName: "SRT subtitles",
        format: "srt",
        srtContent: "translated",
        extras: [],
      }),
    );
  });

  it("lets the target language be changed", async () => {
    setResponse("get_settings", withTranslation);
    setResponse("translate_subtitles", { srt_path: "/p.srt", srt_content: SRT, language: "fr" });
    await renderApp();
    await pickFile();
    await generate();

    const [, target] = screen.getAllByRole("combobox");
    fireEvent.change(target, { target: { value: "fr" } });
    fireEvent.click(screen.getByRole("button", { name: "Translate" }));

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith(
        "translate_subtitles",
        expect.objectContaining({ targetLang: "fr" }),
      ),
    );
  });
});

// ── failures, cancellation, drag and drop, new settings ─────────────────────

const cloudSettings = { ...settings, stt_engine: "groq" };
const withLocalTranslation = { ...settings, translation_engine: "local" };

/** Makes `command` reject with a backend-shaped error. */
function failWith(command: string, error: unknown) {
  mockInvoke.mockImplementation((name: string) =>
    name === command ? Promise.reject(error) : Promise.resolve(responses[name]),
  );
}

describe("App — error messages", () => {
  it("explains a missing ffmpeg instead of showing the raw error", async () => {
    await renderApp();
    await pickFile();
    failWith("run_pipeline", { code: "ffmpeg_missing", message: "ffmpeg was not found" });

    fireEvent.click(screen.getByRole("button", { name: "Generate subtitles" }));

    expect(await screen.findByText(/brew install ffmpeg/)).toBeInTheDocument();
  });

  it("shows the provider's own explanation for a rejected key", async () => {
    await renderApp();
    await pickFile();
    failWith("run_pipeline", {
      code: "api_key_invalid",
      message: "Groq returned HTTP 401: Invalid API Key",
    });

    fireEvent.click(screen.getByRole("button", { name: "Generate subtitles" }));

    expect(await screen.findByText(/refused the request.*Invalid API Key/)).toBeInTheDocument();
  });

  it("is translated when the interface is in French", async () => {
    setResponse("get_settings", { ...settings, language: "fr" });
    await renderApp();
    await pickFileFr();
    failWith("run_pipeline", { code: "no_audio", message: "the file has no audio track" });

    fireEvent.click(screen.getByRole("button", { name: "Générer les sous-titres" }));

    expect(await screen.findByText(/ne contient pas de piste audio/)).toBeInTheDocument();
  });

  it("falls back to the raw message for a code it does not know", async () => {
    await renderApp();
    await pickFile();
    failWith("run_pipeline", { code: "brand_new", message: "something odd happened" });

    fireEvent.click(screen.getByRole("button", { name: "Generate subtitles" }));

    expect(await screen.findByText("something odd happened")).toBeInTheDocument();
  });

  it("announces a failure to assistive technologies", async () => {
    await renderApp();
    await pickFile();
    failWith("run_pipeline", new Error("boom"));

    fireEvent.click(screen.getByRole("button", { name: "Generate subtitles" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("boom");
  });
});

describe("App — cancelling", () => {
  it("offers a cancel button while a run is in progress", async () => {
    await renderApp();
    await pickFile();
    mockInvoke.mockImplementation((command: string) =>
      command === "run_pipeline" ? new Promise(() => {}) : Promise.resolve(responses[command]),
    );
    fireEvent.click(screen.getByRole("button", { name: "Generate subtitles" }));

    fireEvent.click(await screen.findByRole("button", { name: "Cancel" }));

    expect(mockInvoke).toHaveBeenCalledWith("cancel_pipeline");
  });

  it("returns to idle, without an error, when the run was cancelled", async () => {
    await renderApp();
    await pickFile();
    failWith("run_pipeline", { code: "cancelled", message: "cancelled by the user" });

    fireEvent.click(screen.getByRole("button", { name: "Generate subtitles" }));
    await act(async () => {});

    expect(screen.queryByText(/Generation failed/)).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Generate subtitles" })).toBeEnabled();
  });
});

describe("App — regenerating", () => {
  it("brings the generate button back after a result", async () => {
    await renderApp();
    await pickFile();
    await generate();
    expect(screen.queryByRole("button", { name: "Generate subtitles" })).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Regenerate" }));

    expect(screen.getByRole("button", { name: "Generate subtitles" })).toBeInTheDocument();
  });
});

describe("App — the file picker", () => {
  it("passes a localised filter name to the native dialog", async () => {
    await renderApp();
    await pickFile();

    expect(mockInvoke).toHaveBeenCalledWith("pick_file", { filterName: "Video / Audio" });
  });
});

describe("App — drag and drop", () => {
  type DropHandler = (e: { payload: Record<string, unknown> }) => void;
  let handler: DropHandler;

  beforeEach(() => {
    mockWebview.mockImplementation((() => ({
      onDragDropEvent: (cb: DropHandler) => {
        handler = cb;
        return Promise.resolve(() => {});
      },
    })) as unknown as typeof getCurrentWebview);
  });

  it("selects a dropped media file", async () => {
    await renderApp();

    act(() => handler({ payload: { type: "drop", paths: ["/home/me/CLIP.MOV"] } }));

    expect((await screen.findAllByText("CLIP.MOV")).length).toBeGreaterThan(0);
  });

  it("refuses a dropped file that is not media", async () => {
    await renderApp();

    act(() => handler({ payload: { type: "drop", paths: ["/home/me/notes.txt"] } }));

    expect(await screen.findByText(/not supported: notes\.txt/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /browse files/i })).toBeInTheDocument();
  });

  it("highlights the drop zone while a file is dragged over it", async () => {
    await renderApp();

    act(() => handler({ payload: { type: "over", position: { x: 1, y: 1 } } }));

    expect(screen.getByText("Drop the file to select it")).toBeInTheDocument();
    act(() => handler({ payload: { type: "leave" } }));
    expect(screen.queryByText("Drop the file to select it")).not.toBeInTheDocument();
  });

  it("does not swap the file while a run is in progress", async () => {
    await renderApp();
    await pickFile();
    mockInvoke.mockImplementation((command: string) =>
      command === "run_pipeline" ? new Promise(() => {}) : Promise.resolve(responses[command]),
    );
    fireEvent.click(screen.getByRole("button", { name: "Generate subtitles" }));
    await screen.findByRole("button", { name: "Generating…" });

    act(() => handler({ payload: { type: "drop", paths: ["/home/me/other.mov"] } }));

    expect(screen.queryByText("other.mov")).not.toBeInTheDocument();
    expect(screen.getAllByText("clip.mp4").length).toBe(2);
  });

  it("still works when drag and drop is unavailable", async () => {
    mockWebview.mockImplementation((() => {
      throw new Error("not inside Tauri");
    }) as unknown as typeof getCurrentWebview);

    await renderApp();

    expect(screen.getByRole("button", { name: /browse files/i })).toBeInTheDocument();
  });
});

describe("App — transcription settings", () => {
  async function openSettings() {
    fireEvent.keyDown(window, { key: ",", ctrlKey: true });
  }

  it("persists the local whisper model", async () => {
    await renderApp();
    await openSettings();

    fireEvent.change(screen.getByLabelText("Whisper model"), { target: { value: "medium" } });

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("set_settings", {
        settings: { ...settings, whisper_model: "medium" },
      }),
    );
  });

  it("offers the model choice only for the local engine", async () => {
    setResponse("get_settings", cloudSettings);
    await renderApp();
    await openSettings();

    expect(screen.queryByLabelText("Whisper model")).not.toBeInTheDocument();
  });

  it("persists a forced spoken language", async () => {
    await renderApp();
    await openSettings();

    fireEvent.change(screen.getByLabelText("Spoken language"), { target: { value: "fr" } });

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("set_settings", {
        settings: { ...settings, stt_language: "fr" },
      }),
    );
  });

  it("states that a cloud engine uploads the audio", async () => {
    setResponse("get_settings", cloudSettings);
    await renderApp();
    await openSettings();

    expect(screen.getByText(/uploads your audio/)).toBeInTheDocument();
  });

  it("does not claim that for the local engine", async () => {
    await renderApp();
    await openSettings();

    expect(screen.queryByText(/uploads your audio/)).not.toBeInTheDocument();
  });

  it("writes the Azure region once the user stops typing", async () => {
    setResponse("get_settings", { ...settings, translation_engine: "azure" });
    await renderApp();
    await openSettings();
    const input = screen.getByLabelText("Azure region");

    fireEvent.change(input, { target: { value: "w" } });
    fireEvent.change(input, { target: { value: "westeurope" } });

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("set_settings", {
        settings: {
          ...settings,
          translation_engine: "azure",
          azure_translator_region: "westeurope",
        },
      }),
    );
    const writes = mockInvoke.mock.calls.filter(([c]) => c === "set_settings");
    expect(writes).toHaveLength(1);
  });
});

describe("App — API keys", () => {
  it("warns before generating when the cloud engine has no key", async () => {
    setResponse("get_settings", cloudSettings);
    await renderApp();
    await pickFile();

    expect(
      await screen.findByText(/Add your API key in Settings before generating/),
    ).toBeInTheDocument();
    // A warning, not a block: the backend still reports precisely if it is really missing.
    expect(screen.getByRole("button", { name: "Generate subtitles" })).toBeEnabled();
  });

  it("does not warn when the key is saved", async () => {
    setResponse("get_settings", cloudSettings);
    setResponse("has_api_key", true);
    await renderApp();
    await pickFile();
    await act(async () => {});

    expect(screen.queryByText(/before generating/)).not.toBeInTheDocument();
  });

  it("does not warn for the local engine", async () => {
    await renderApp();
    await pickFile();
    await act(async () => {});

    expect(screen.queryByText(/before generating/)).not.toBeInTheDocument();
  });

  it("lets a saved key be removed", async () => {
    setResponse("get_settings", cloudSettings);
    setResponse("has_api_key", true);
    await renderApp();
    fireEvent.keyDown(window, { key: ",", ctrlKey: true });

    fireEvent.click(await screen.findByRole("button", { name: "Remove" }));

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("delete_api_key", { keyName: "groq_api_key" }),
    );
  });

  it("reports a keyring failure instead of failing silently", async () => {
    setResponse("get_settings", cloudSettings);
    await renderApp();
    fireEvent.keyDown(window, { key: ",", ctrlKey: true });
    mockInvoke.mockImplementation((command: string) =>
      command === "set_api_key"
        ? Promise.reject({ code: null, message: "no secret service available" })
        : Promise.resolve(responses[command]),
    );

    fireEvent.change(screen.getByLabelText("API key"), { target: { value: "gsk_abc" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    expect(await screen.findByText(/no secret service available/)).toBeInTheDocument();
  });
});

describe("App — translation languages", () => {
  it("blocks translating into the language that was just spoken", async () => {
    setResponse("get_settings", { ...settings, translation_engine: "deep_l" });
    await renderApp();
    await pickFile();
    await generate({ language: "fr" });
    fireEvent.change(screen.getAllByRole("combobox")[1], { target: { value: "fr" } });

    expect(screen.getByRole("button", { name: "Translate" })).toBeDisabled();
    expect(screen.getByText(/languages are the same/)).toBeInTheDocument();
  });

  it("starts on another target when the speech is already in the default one", async () => {
    setResponse("get_settings", { ...settings, translation_engine: "deep_l" });
    await renderApp();
    await pickFile();
    await generate({ language: "en" });

    expect(screen.getByRole("button", { name: "Translate" })).toBeEnabled();
    expect(screen.getByLabelText("Target language")).toHaveValue("fr");
  });

  it("gives the local translator the detected language when left on Auto", async () => {
    setResponse("get_settings", withLocalTranslation);
    setResponse("translate_subtitles", { srt_path: "/p.srt", srt_content: SRT, language: "fr" });
    await renderApp();
    await pickFile();
    await generate({ language: "fr" });
    fireEvent.change(screen.getByLabelText("Source language"), { target: { value: "auto" } });
    fireEvent.change(screen.getByLabelText("Target language"), { target: { value: "en" } });

    fireEvent.click(screen.getByRole("button", { name: "Translate" }));

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith(
        "translate_subtitles",
        expect.objectContaining({ sourceLang: "fr", targetLang: "en" }),
      ),
    );
  });

  it("keeps Auto when the detected language is not a supported one", async () => {
    setResponse("get_settings", { ...settings, translation_engine: "deep_l" });
    await renderApp();
    await pickFile();
    await generate({ language: "unknown" });

    expect(screen.getByLabelText("Source language")).toHaveValue("auto");
  });

  it("names the languages in the interface language", async () => {
    setResponse("get_settings", { ...settings, language: "fr", translation_engine: "deep_l" });
    await renderApp();
    await pickFileFr();
    setResponse("run_pipeline", {
      srt_path: "/home/me/videos/clip.srt",
      srt_content: SRT,
      language: "en",
    });
    fireEvent.click(screen.getByRole("button", { name: "Générer les sous-titres" }));
    await screen.findByText("clip.srt");

    expect(screen.getByLabelText("Langue cible")).toHaveDisplayValue("français");
  });
});

describe("App — progress", () => {
  it("shows the audio extraction before the model download, as the backend runs them", async () => {
    await renderApp();
    await pickFile();
    mockInvoke.mockImplementation((command: string) =>
      command === "run_pipeline" ? new Promise(() => {}) : Promise.resolve(responses[command]),
    );
    fireEvent.click(screen.getByRole("button", { name: "Generate subtitles" }));
    await screen.findByRole("button", { name: "Generating…" });

    const labels = [...document.querySelectorAll(".step-label")].map((n) => n.textContent);

    expect(labels).toEqual(["Audio", "Model", "Transcript", "Subtitles"]);
  });

  it("exposes the progress to assistive technologies", async () => {
    await renderApp();
    await pickFile();
    mockInvoke.mockImplementation((command: string) =>
      command === "run_pipeline" ? new Promise(() => {}) : Promise.resolve(responses[command]),
    );
    fireEvent.click(screen.getByRole("button", { name: "Generate subtitles" }));
    await screen.findByRole("button", { name: "Generating…" });

    act(() => onProgress?.({ payload: { stage: "transcribe", fraction: 0.42 } }));

    expect(screen.getByRole("progressbar", { name: "Progress" })).toHaveAttribute(
      "aria-valuenow",
      "42",
    );
  });
});

describe("App — header", () => {
  it("carries the product name", async () => {
    await renderApp();

    expect(screen.getByRole("heading", { name: "LightGenSubz" })).toBeInTheDocument();
  });
});

// ── audio tracks, ASS export, editing ───────────────────────────────────────

const TRACKS = [
  { position: 0, language: "fra", title: "Français", codec: "aac", channels: 2 },
  { position: 1, language: "eng", title: null, codec: "ac3", channels: 6 },
];

describe("App — audio tracks", () => {
  it("offers a choice only when the file has several audio tracks", async () => {
    await renderApp();
    await pickFile();

    expect(screen.queryByLabelText("Audio track")).not.toBeInTheDocument();
  });

  it("lists the tracks with their language and codec", async () => {
    setResponse("list_audio_tracks", TRACKS);
    await renderApp();
    await pickFile();

    const select = await screen.findByLabelText("Audio track");

    expect(select).toHaveTextContent("#1 — French · Français (aac, 2ch)");
    expect(select).toHaveTextContent("#2 — English (ac3, 6ch)");
  });

  it("sends the chosen track to the backend", async () => {
    setResponse("list_audio_tracks", TRACKS);
    await renderApp();
    await pickFile();
    fireEvent.change(await screen.findByLabelText("Audio track"), { target: { value: "1" } });

    await generate();

    expect(mockInvoke).toHaveBeenCalledWith("run_pipeline", {
      inputPath: "/home/me/videos/clip.mp4",
      audioTrack: 1,
    });
  });

  it("keeps working when the tracks cannot be listed", async () => {
    mockInvoke.mockImplementation((command: string) =>
      command === "list_audio_tracks"
        ? Promise.reject(new Error("ffprobe missing"))
        : Promise.resolve(responses[command]),
    );
    await renderApp();
    await pickFile();

    expect(screen.getByRole("button", { name: "Generate subtitles" })).toBeEnabled();
  });
});

describe("App — ASS export", () => {
  it("exports the transcript as an ASS script", async () => {
    await renderApp();
    await pickFile();
    await generate();

    exportAs("ass");

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith(
        "save_subtitle_as",
        expect.objectContaining({ format: "ass", filterName: "ASS subtitles" }),
      ),
    );
  });
});

describe("App — editing cues", () => {
  it("applies the edited texts through the backend and shows the result", async () => {
    setResponse("update_subtitles", {
      srt_content: ["1", "00:00:00,000 --> 00:00:02,000", "Hello world"].join("\n"),
    });
    await renderApp();
    await pickFile();
    await generate();

    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
    fireEvent.change(screen.getByLabelText("Text of cue 1"), { target: { value: "Hello world" } });
    fireEvent.click(screen.getByRole("button", { name: "Apply changes" }));

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("update_subtitles", {
        srtContent: SRT,
        texts: ["Hello world"],
      }),
    );
    expect(await screen.findByText("Hello world")).toBeInTheDocument();
    expect(screen.queryByLabelText("Text of cue 1")).not.toBeInTheDocument();
  });

  it("cannot apply before anything changed", async () => {
    await renderApp();
    await pickFile();
    await generate();

    fireEvent.click(screen.getByRole("button", { name: "Edit" }));

    expect(screen.getByRole("button", { name: "Apply changes" })).toBeDisabled();
  });

  it("drops the edits on discard", async () => {
    await renderApp();
    await pickFile();
    await generate();
    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
    fireEvent.change(screen.getByLabelText("Text of cue 1"), { target: { value: "Changed" } });

    fireEvent.click(screen.getByRole("button", { name: "Discard" }));

    expect(screen.getByText("Hello there")).toBeInTheDocument();
    expect(mockInvoke).not.toHaveBeenCalledWith("update_subtitles", expect.anything());
  });

  it("reports a failure and keeps the editor open", async () => {
    await renderApp();
    await pickFile();
    await generate();
    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
    fireEvent.change(screen.getByLabelText("Text of cue 1"), { target: { value: "Changed" } });
    mockInvoke.mockImplementation((command: string) =>
      command === "update_subtitles"
        ? Promise.reject({ code: null, message: "disk full" })
        : Promise.resolve(responses[command]),
    );

    fireEvent.click(screen.getByRole("button", { name: "Apply changes" }));

    expect(await screen.findByText(/Could not apply the changes: disk full/)).toBeInTheDocument();
    expect(screen.getByLabelText("Text of cue 1")).toHaveValue("Changed");
  });

  it("edits the translation too", async () => {
    setResponse("get_settings", { ...settings, translation_engine: "deep_l" });
    setResponse("translate_subtitles", {
      srt_path: "/home/me/videos/clip.fr.srt",
      srt_content: ["1", "00:00:00,000 --> 00:00:02,000", "Bonjour"].join("\n"),
      language: "fr",
    });
    await renderApp();
    await pickFile();
    await generate();
    fireEvent.click(screen.getByRole("button", { name: "Translate" }));
    await screen.findByText("Bonjour");

    fireEvent.click(screen.getAllByRole("button", { name: "Edit" })[1]);

    expect(screen.getByLabelText("Text of cue 1")).toHaveValue("Bonjour");
  });
});

describe("App — saving feedback", () => {
  it("confirms where the file was saved", async () => {
    await renderApp();
    await pickFile();
    await generate();

    exportAs("srt");

    expect(await screen.findByText("Saved to /home/me/final.srt")).toBeInTheDocument();
  });

  it("says nothing when the save dialog is dismissed", async () => {
    setResponse("save_subtitle_as", null);
    await renderApp();
    await pickFile();
    await generate();

    exportAs("srt");

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("save_subtitle_as", expect.anything()),
    );
    expect(screen.queryByText(/Saved to/)).not.toBeInTheDocument();
    expect(screen.queryByText(/Could not save the file/)).not.toBeInTheDocument();
  });

  it("reports a failed save instead of staying silent", async () => {
    await renderApp();
    await pickFile();
    await generate();
    failWith("save_subtitle_as", { code: null, message: "could not write /usb/x.srt: read-only" });

    exportAs("vtt");

    expect(await screen.findByText(/Could not save the file: .*read-only/)).toBeInTheDocument();
    expect(screen.getByRole("alert", { name: "" })).toBeInTheDocument();
  });

  it("reports a failed save of a translation next to the translation", async () => {
    setResponse("get_settings", { ...settings, translation_engine: "local" });
    setResponse("translate_subtitles", { srt_path: "/p.fr.srt", srt_content: SRT, language: "fr" });
    await renderApp();
    await pickFile();
    await generate();
    fireEvent.click(screen.getByRole("button", { name: "Translate" }));
    await screen.findByText("p.fr.srt");
    failWith("save_subtitle_as", { code: null, message: "disk full" });

    exportAs("srt", "translation");

    expect(await screen.findByText(/Could not save the file: disk full/)).toBeInTheDocument();
  });
});

describe("App — edits", () => {
  it("asks before removing the cues that were emptied", async () => {
    setResponse("update_subtitles", {
      srt_content: ["1", "00:00:00,000 --> 00:00:02,000", "Hello there"].join("\n"),
    });
    await renderApp();
    await pickFile();
    await generate({ srt_content: SRT_TWO });
    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
    fireEvent.change(screen.getByLabelText("Text of cue 2"), { target: { value: "" } });

    expect(screen.getByText(/1 empty cue\(s\) will be removed/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Apply changes" }));
    fireEvent.click(screen.getByRole("button", { name: "Remove them and apply" }));

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith(
        "update_subtitles",
        expect.objectContaining({ texts: ["Hello there", ""] }),
      ),
    );
  });

  it("does not send anything on the first click when a cue was emptied", async () => {
    await renderApp();
    await pickFile();
    await generate({ srt_content: SRT_TWO });
    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
    fireEvent.change(screen.getByLabelText("Text of cue 2"), { target: { value: "  " } });

    fireEvent.click(screen.getByRole("button", { name: "Apply changes" }));

    expect(mockInvoke).not.toHaveBeenCalledWith("update_subtitles", expect.anything());
    expect(screen.getByRole("button", { name: "Remove them and apply" })).toBeInTheDocument();
  });

  it("shows the lines a cue already has", async () => {
    await renderApp();
    await pickFile();
    await generate({ srt_content: "1\n00:00:00,000 --> 00:00:02,000\n- Hi!\n- Hello." });
    fireEvent.click(screen.getByRole("button", { name: "Edit" }));

    expect(screen.getByLabelText("Text of cue 1")).toHaveValue("- Hi!\n- Hello.");
  });
});

describe("App — settings are saved in order and reported when they fail", () => {
  it("shows a failed save of the settings", async () => {
    await renderApp();
    fireEvent.keyDown(window, { key: ",", ctrlKey: true });
    failWith("set_settings", { code: null, message: "settings.json is read-only" });

    fireEvent.change(screen.getByLabelText("Transcription engine"), { target: { value: "groq" } });

    expect(await screen.findByText(/Could not save the settings: .*read-only/)).toBeInTheDocument();
  });

  it("sends two quick changes one after the other, the second built on the first", async () => {
    await renderApp();
    fireEvent.keyDown(window, { key: ",", ctrlKey: true });

    fireEvent.change(screen.getByLabelText("Transcription engine"), { target: { value: "groq" } });
    fireEvent.change(screen.getByLabelText("Translation"), { target: { value: "deep_l" } });

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("set_settings", {
        settings: expect.objectContaining({ stt_engine: "groq", translation_engine: "deep_l" }),
      }),
    );
  });
});

describe("App — cancelling feedback and timing", () => {
  it("says the stop was heard while the run winds down", async () => {
    await renderApp();
    await pickFile();
    mockInvoke.mockImplementation((command: string) =>
      command === "run_pipeline" ? new Promise(() => {}) : Promise.resolve(responses[command]),
    );
    fireEvent.click(screen.getByRole("button", { name: "Generate subtitles" }));
    fireEvent.click(await screen.findByRole("button", { name: "Cancel" }));

    expect(await screen.findByRole("button", { name: "Cancelling…" })).toBeDisabled();
  });

  it("shows the time elapsed in the current stage", async () => {
    // Only the clock and the interval: Testing Library's own waiting needs real timeouts.
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval", "Date"] });
    try {
      await renderApp();
      await pickFile();
      mockInvoke.mockImplementation((command: string) =>
        command === "run_pipeline" ? new Promise(() => {}) : Promise.resolve(responses[command]),
      );
      fireEvent.click(screen.getByRole("button", { name: "Generate subtitles" }));
      await act(async () => {
        onProgress?.({ payload: { stage: "transcribe", fraction: 0.5 } });
      });
      await act(async () => {
        await vi.advanceTimersByTimeAsync(10_000);
      });

      expect(screen.getByText(/Elapsed 0:10 · about 0:10 left/)).toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });
});
