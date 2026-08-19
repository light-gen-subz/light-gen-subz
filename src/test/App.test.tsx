import { render, screen, fireEvent, waitFor, act } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import { listen } from "@tauri-apps/api/event";
import { save } from "@tauri-apps/plugin-dialog";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import App from "../App";

const mockInvoke = vi.mocked(invoke);
const mockListen = vi.mocked(listen);
const mockSave = vi.mocked(save);
const mockCheck = vi.mocked(check);
const mockRelaunch = vi.mocked(relaunch);

const SRT = ["1", "00:00:00,000 --> 00:00:02,000", "Hello there"].join("\n");
const SRT_TWO = [SRT, ["2", "00:00:02,000 --> 00:00:04,000", "General Kenobi"].join("\n")].join(
  "\n\n",
);

const settings = {
  stt_engine: "local",
  translation_engine: "none",
  azure_translator_region: "",
  language: "en",
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

async function renderApp() {
  render(<App />);
  // settle the two settings/languages effects
  await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("get_settings"));
  await act(async () => {});
}

/** Picks a file and waits for the chip to appear. */
async function pickFile(path = "/home/me/videos/clip.mp4") {
  setResponse("pick_file", path);
  fireEvent.click(screen.getByRole("button", { name: /choose a video or audio file/i }));
  await screen.findByText("clip.mp4");
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
  await screen.findByText("/home/me/videos/clip.srt");
}

beforeEach(() => {
  vi.clearAllMocks();
  onProgress = undefined;
  responses = {
    get_settings: { ...settings },
    list_languages: languages,
    has_api_key: false,
    pick_file: null,
    set_settings: undefined,
    set_api_key: undefined,
    save_subtitle: undefined,
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
  mockSave.mockResolvedValue(null);
});

describe("App — initial state", () => {
  it("offers the file picker and reports Idle", async () => {
    await renderApp();

    expect(screen.getByRole("button", { name: /choose a video or audio file/i })).toBeInTheDocument();
    expect(screen.getByText("Idle")).toBeInTheDocument();
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

    expect(screen.getByText("clip.mp4")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Generate subtitles" })).toBeInTheDocument();
  });

  it("keeps the dropzone when the picker is dismissed", async () => {
    await renderApp();
    setResponse("pick_file", null);

    fireEvent.click(screen.getByRole("button", { name: /choose a video or audio file/i }));
    await act(async () => {});

    expect(
      screen.getByRole("button", { name: /choose a video or audio file/i }),
    ).toBeInTheDocument();
  });

  it("lets the file be swapped through the Change button", async () => {
    await renderApp();
    await pickFile();

    setResponse("pick_file", "/home/me/other.mov");
    fireEvent.click(screen.getByRole("button", { name: "Change" }));

    expect(await screen.findByText("other.mov")).toBeInTheDocument();
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
    expect(screen.getByText("Done")).toBeInTheDocument();
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
    expect(screen.getByText("Error")).toBeInTheDocument();
  });

  it("passes the chosen file to the backend", async () => {
    await renderApp();
    await pickFile();
    await generate();

    expect(mockInvoke).toHaveBeenCalledWith("run_pipeline", {
      inputPath: "/home/me/videos/clip.mp4",
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
    await screen.findByText("Running");

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
    await screen.findByText("Running");

    const labels = [...document.querySelectorAll(".step-label")].map((n) => n.textContent);
    expect(labels).toEqual(["Audio", "Transcript", "Subtitles"]);

    await act(async () => {
      resolvePipeline({ srt_path: "/p.srt", srt_content: SRT, language: "en" });
    });
  });
});

describe("App — saving", () => {
  it("writes the transcript to the chosen destination", async () => {
    await renderApp();
    await pickFile();
    await generate();
    mockSave.mockResolvedValue("/home/me/final.srt");

    fireEvent.click(screen.getByRole("button", { name: "Save as…" }));

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("save_subtitle", {
        destPath: "/home/me/final.srt",
        content: SRT,
      }),
    );
  });

  it("writes nothing when the save dialog is dismissed", async () => {
    await renderApp();
    await pickFile();
    await generate();
    mockSave.mockResolvedValue(null);

    fireEvent.click(screen.getByRole("button", { name: "Save as…" }));
    await act(async () => {});

    expect(mockInvoke).not.toHaveBeenCalledWith("save_subtitle", expect.anything());
  });
});

describe("App — settings panel", () => {
  it("is hidden until the settings button is pressed", async () => {
    await renderApp();
    expect(screen.queryByText("Transcription engine")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Settings" }));

    expect(screen.getByText("Transcription engine")).toBeInTheDocument();
  });

  it("persists a change of transcription engine", async () => {
    await renderApp();
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));

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
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));

    fireEvent.change(screen.getByDisplayValue("Off"), { target: { value: "local" } });

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("set_settings", {
        settings: { ...settings, translation_engine: "local" },
      }),
    );
  });

  it("asks for a region only for Azure", async () => {
    setResponse("get_settings", { ...settings, translation_engine: "azure" });
    await renderApp();

    fireEvent.click(screen.getByRole("button", { name: "Settings" }));

    expect(screen.getByText("Azure region")).toBeInTheDocument();
  });

  it("persists the Azure region as it is typed", async () => {
    setResponse("get_settings", { ...settings, translation_engine: "azure" });
    await renderApp();
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));

    fireEvent.change(screen.getByPlaceholderText("westeurope"), {
      target: { value: "westeurope" },
    });

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("set_settings", {
        settings: { ...settings, translation_engine: "azure", azure_translator_region: "westeurope" },
      }),
    );
  });
});

describe("App — language", () => {
  async function openSettings(lang = "en") {
    setResponse("get_settings", { ...settings, language: lang });
    await renderApp();
    fireEvent.click(screen.getByRole("button", { name: lang === "fr" ? "Réglages" : "Settings" }));
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

    fireEvent.change(screen.getByDisplayValue("English"), { target: { value: "fr" } });

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("set_settings", {
        settings: { ...settings, language: "fr" },
      }),
    );
  });

  it("switches the interface without waiting for a restart", async () => {
    await openSettings();

    fireEvent.change(screen.getByDisplayValue("English"), { target: { value: "fr" } });

    expect(await screen.findByText("Moteur de transcription")).toBeInTheDocument();
  });

  it("translates the body, not just the settings", async () => {
    setResponse("get_settings", { ...settings, language: "fr" });
    await renderApp();

    expect(
      screen.getByRole("button", { name: /choisissez un fichier vidéo ou audio/i }),
    ).toBeInTheDocument();
  });

  it("tells the page which language it is in, for the spell checker and screen readers", async () => {
    setResponse("get_settings", { ...settings, language: "fr" });
    await renderApp();

    expect(document.documentElement.lang).toBe("fr");
  });
});

describe("App — version", () => {
  it("shows the running version in the settings panel", async () => {
    await renderApp();

    fireEvent.click(screen.getByRole("button", { name: "Settings" }));

    expect(await screen.findByText("v0.2.3")).toBeInTheDocument();
  });

  it("does not leave the app unusable when the version cannot be read", async () => {
    vi.mocked(getVersion).mockRejectedValueOnce(new Error("no ipc"));
    await renderApp();

    fireEvent.click(screen.getByRole("button", { name: "Settings" }));

    expect(screen.getByText("Transcription engine")).toBeInTheDocument();
  });
});

describe("App — API key field", () => {
  it("appears for a cloud engine and reports a stored key", async () => {
    setResponse("get_settings", { ...settings, stt_engine: "groq" });
    setResponse("has_api_key", true);
    await renderApp();

    fireEvent.click(screen.getByRole("button", { name: "Settings" }));

    expect(await screen.findByText(/API key \(saved\)/)).toBeInTheDocument();
  });

  it("stores a trimmed key and clears the input", async () => {
    setResponse("get_settings", { ...settings, stt_engine: "groq" });
    await renderApp();
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));

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
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));

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
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));

    fireEvent.click(screen.getByRole("button", { name: "Check for updates" }));

    expect(await screen.findByText("You're up to date")).toBeInTheDocument();
  });

  it("installs an available update and relaunches", async () => {
    const downloadAndInstall = vi.fn().mockResolvedValue(undefined);
    mockCheck.mockResolvedValue({ downloadAndInstall } as never);
    await renderApp();
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));

    fireEvent.click(screen.getByRole("button", { name: "Check for updates" }));

    await waitFor(() => expect(downloadAndInstall).toHaveBeenCalled());
    await waitFor(() => expect(mockRelaunch).toHaveBeenCalled());
  });

  it("shows the reason when the check fails", async () => {
    mockCheck.mockRejectedValue(new Error("network down"));
    await renderApp();
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));

    fireEvent.click(screen.getByRole("button", { name: "Check for updates" }));

    expect(await screen.findByText(/Update check failed: .*network down/)).toBeInTheDocument();
  });
});

describe("App — translation", () => {
  const withTranslation = { ...settings, translation_engine: "local" };

  it("stays hidden while translation is switched off", async () => {
    await renderApp();
    await pickFile();
    await generate();

    expect(screen.queryByRole("button", { name: "Translate" })).not.toBeInTheDocument();
  });

  it("offers translation once an engine is configured", async () => {
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
    });
    await renderApp();
    await pickFile();
    await generate();

    fireEvent.click(screen.getByRole("button", { name: "Translate" }));

    expect(await screen.findByText("Bonjour")).toBeInTheDocument();
    expect(screen.getByText("/home/me/videos/clip.fr.srt")).toBeInTheDocument();
  });

  it("sends null as the source language when set to auto", async () => {
    setResponse("get_settings", withTranslation);
    setResponse("translate_subtitles", { srt_path: "/p.srt", srt_content: SRT });
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
    setResponse("translate_subtitles", { srt_path: "/p.fr.srt", srt_content: "translated" });
    await renderApp();
    await pickFile();
    await generate();
    fireEvent.click(screen.getByRole("button", { name: "Translate" }));
    await screen.findByText("/p.fr.srt");

    mockSave.mockResolvedValue("/home/me/final.fr.srt");
    fireEvent.click(screen.getAllByRole("button", { name: "Save as…" })[1]);

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("save_subtitle", {
        destPath: "/home/me/final.fr.srt",
        content: "translated",
      }),
    );
  });

  it("lets the target language be changed", async () => {
    setResponse("get_settings", withTranslation);
    setResponse("translate_subtitles", { srt_path: "/p.srt", srt_content: SRT });
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
