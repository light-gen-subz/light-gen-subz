import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import { listen } from "@tauri-apps/api/event";
import { save } from "@tauri-apps/plugin-dialog";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { getPlural, getT, type Lang } from "./i18n";
import "./App.css";

type UpdateStatus = "idle" | "checking" | "up_to_date" | "installing" | "error";

type Stage =
  | "download_model"
  | "extract_audio"
  | "transcribe"
  | "write_subtitles"
  | "download_translation_model"
  | "translate";

type PipelineProgress = {
  stage: Stage;
  fraction: number;
};

type PipelineOutput = {
  srt_path: string;
  srt_content: string;
  language: string;
};

type TranslationOutput = {
  srt_path: string;
  srt_content: string;
};

export type Cue = {
  index: number;
  start: string;
  end: string;
  text: string;
};

type SttEngineChoice = "local" | "groq" | "open_ai" | "deepgram" | "assembly_ai";
type TranslationEngineChoice = "none" | "local" | "deep_l" | "open_ai" | "google" | "azure";

type Settings = {
  stt_engine: SttEngineChoice;
  translation_engine: TranslationEngineChoice;
  azure_translator_region: string;
  language: Lang;
};

type LanguageInfo = {
  code: string;
  flores_code: string;
  name: string;
};

type EngineOption<T extends string> = {
  value: T;
  labelKey: string;
  keyName?: string;
  keyPlaceholder?: string;
  needsRegion?: boolean;
};

const STT_ENGINES: EngineOption<SttEngineChoice>[] = [
  { value: "local", labelKey: "stt.local" },
  { value: "groq", labelKey: "stt.groq", keyName: "groq_api_key", keyPlaceholder: "gsk_..." },
  { value: "open_ai", labelKey: "stt.openai", keyName: "openai_api_key", keyPlaceholder: "sk-..." },
  { value: "deepgram", labelKey: "stt.deepgram", keyName: "deepgram_api_key", keyPlaceholder: "API key" },
  {
    value: "assembly_ai",
    labelKey: "stt.assemblyai",
    keyName: "assemblyai_api_key",
    keyPlaceholder: "API key",
  },
];

const TRANSLATION_ENGINES: EngineOption<TranslationEngineChoice>[] = [
  { value: "none", labelKey: "tr.none" },
  { value: "local", labelKey: "tr.local" },
  { value: "deep_l", labelKey: "tr.deepl", keyName: "deepl_api_key", keyPlaceholder: "xxxxxxxx-...:fx" },
  { value: "open_ai", labelKey: "tr.openai", keyName: "openai_api_key", keyPlaceholder: "sk-..." },
  {
    value: "google",
    labelKey: "tr.google",
    keyName: "google_translate_api_key",
    keyPlaceholder: "API key",
  },
  {
    value: "azure",
    labelKey: "tr.azure",
    keyName: "azure_translator_key",
    keyPlaceholder: "API key",
    needsRegion: true,
  },
];

const TRANSCRIBE_STAGES: { key: Stage; labelKey: string }[] = [
  { key: "download_model", labelKey: "stage.model" },
  { key: "extract_audio", labelKey: "stage.audio" },
  { key: "transcribe", labelKey: "stage.transcript" },
  { key: "write_subtitles", labelKey: "stage.subtitles" },
];

export function fileName(path: string): string {
  return path.split(/[\\/]/).pop() ?? path;
}

export function parseSrt(content: string): Cue[] {
  return content
    .trim()
    .split(/\r?\n\r?\n/)
    .map((block) => {
      const lines = block.split(/\r?\n/);
      const [index, timing, ...textLines] = lines;
      const [start, end] = (timing ?? "").split(" --> ");
      return {
        index: Number(index) || 0,
        start: (start ?? "").trim(),
        end: (end ?? "").trim(),
        text: textLines.join(" ").trim(),
      };
    })
    .filter((cue) => cue.text.length > 0);
}

function CueList({ cues }: { cues: Cue[] }) {
  return (
    <ol className="cue-list">
      {cues.map((cue) => (
        <li className="cue" key={cue.index}>
          <span className="cue-index">{cue.index}</span>
          <div className="cue-body">
            <span className="cue-time">
              {cue.start} <span className="cue-arrow">→</span> {cue.end}
            </span>
            <p className="cue-text">{cue.text}</p>
          </div>
        </li>
      ))}
    </ol>
  );
}

function ApiKeyField({
  keyName,
  placeholder,
  t,
}: {
  keyName: string;
  placeholder: string;
  t: ReturnType<typeof getT>;
}) {
  const [value, setValue] = useState("");
  const [hasKey, setHasKey] = useState(false);

  useEffect(() => {
    invoke<boolean>("has_api_key", { keyName }).then(setHasKey);
  }, [keyName]);

  async function saveKey() {
    if (!value.trim()) return;
    await invoke("set_api_key", { keyName, value: value.trim() });
    setHasKey(true);
    setValue("");
  }

  return (
    <div className="settings-row">
      <span className="settings-label">
        {t("settings.apiKey")} {hasKey && `(${t("settings.apiKeySaved")})`}
      </span>
      <div className="cmd-row">
        <input
          className="settings-input"
          type="password"
          placeholder={placeholder}
          value={value}
          onChange={(e) => setValue(e.currentTarget.value)}
        />
        <button className="btn btn-ghost" onClick={saveKey} type="button">
          {t("settings.save")}
        </button>
      </div>
    </div>
  );
}

function App() {
  const [inputPath, setInputPath] = useState<string | null>(null);
  const [progress, setProgress] = useState<PipelineProgress | null>(null);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<PipelineOutput | null>(null);

  const [settings, setSettings] = useState<Settings | null>(null);
  const [languages, setLanguages] = useState<LanguageInfo[]>([]);
  const [showSettings, setShowSettings] = useState(false);
  const [updateStatus, setUpdateStatus] = useState<UpdateStatus>("idle");
  const [updateError, setUpdateError] = useState<string | null>(null);
  const [version, setVersion] = useState("");

  const [sourceLang, setSourceLang] = useState("auto");
  const [targetLang, setTargetLang] = useState("en");
  const [translating, setTranslating] = useState(false);
  const [translationError, setTranslationError] = useState<string | null>(null);
  const [translation, setTranslation] = useState<TranslationOutput | null>(null);

  useEffect(() => {
    const unlisten = listen<PipelineProgress>("pipeline-progress", (event) => {
      setProgress(event.payload);
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  useEffect(() => {
    invoke<Settings>("get_settings").then(setSettings);
    invoke<LanguageInfo[]>("list_languages").then(setLanguages);
    getVersion().then(setVersion).catch(() => {});
  }, []);

  const lang: Lang = settings?.language ?? "en";
  const t = useMemo(() => getT(lang), [lang]);
  const plural = useMemo(() => getPlural(lang), [lang]);

  useEffect(() => {
    document.documentElement.lang = lang;
  }, [lang]);

  const cues = useMemo(() => (result ? parseSrt(result.srt_content) : []), [result]);
  const translatedCues = useMemo(
    () => (translation ? parseSrt(translation.srt_content) : []),
    [translation],
  );

  const transcribeStages = useMemo(
    () =>
      settings?.stt_engine === "local"
        ? TRANSCRIBE_STAGES
        : TRANSCRIBE_STAGES.filter((s) => s.key !== "download_model"),
    [settings],
  );
  const stageIndex = progress ? transcribeStages.findIndex((s) => s.key === progress.stage) : -1;
  const isTranslateStage =
    progress?.stage === "translate" || progress?.stage === "download_translation_model";

  const currentSttEngine = STT_ENGINES.find((o) => o.value === settings?.stt_engine);
  const currentTranslationEngine = TRANSLATION_ENGINES.find(
    (o) => o.value === settings?.translation_engine,
  );

  async function pickFile() {
    const path = await invoke<string | null>("pick_file");
    if (path) {
      setInputPath(path);
      setResult(null);
      setError(null);
      setTranslation(null);
      setTranslationError(null);
    }
  }

  async function generate() {
    if (!inputPath) return;
    setRunning(true);
    setError(null);
    setResult(null);
    setTranslation(null);
    setTranslationError(null);
    setProgress(null);
    try {
      const output = await invoke<PipelineOutput>("run_pipeline", { inputPath });
      setResult(output);
      if (output.language) {
        setSourceLang(output.language.slice(0, 2).toLowerCase());
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setRunning(false);
    }
  }

  async function saveAs() {
    if (!result) return;
    const dest = await save({
      defaultPath: result.srt_path,
      filters: [{ name: "SRT subtitles", extensions: ["srt"] }],
    });
    if (dest) {
      await invoke("save_subtitle", { destPath: dest, content: result.srt_content });
    }
  }

  async function saveTranslationAs() {
    if (!translation) return;
    const dest = await save({
      defaultPath: translation.srt_path,
      filters: [{ name: "SRT subtitles", extensions: ["srt"] }],
    });
    if (dest) {
      await invoke("save_subtitle", { destPath: dest, content: translation.srt_content });
    }
  }

  async function runTranslation() {
    if (!result) return;
    setTranslating(true);
    setTranslationError(null);
    setTranslation(null);
    setProgress(null);
    try {
      const output = await invoke<TranslationOutput>("translate_subtitles", {
        srtPath: result.srt_path,
        srtContent: result.srt_content,
        sourceLang: sourceLang === "auto" ? null : sourceLang,
        targetLang,
      });
      setTranslation(output);
    } catch (e) {
      setTranslationError(String(e));
    } finally {
      setTranslating(false);
    }
  }

  async function updateSettings(patch: Partial<Settings>) {
    if (!settings) return;
    const next = { ...settings, ...patch };
    setSettings(next);
    await invoke("set_settings", { settings: next });
  }

  async function checkForUpdates() {
    setUpdateStatus("checking");
    setUpdateError(null);
    try {
      const update = await check();
      if (!update) {
        setUpdateStatus("up_to_date");
        return;
      }
      setUpdateStatus("installing");
      await update.downloadAndInstall();
      await relaunch();
    } catch (e) {
      setUpdateStatus("error");
      setUpdateError(String(e));
    }
  }

  const status: "idle" | "running" | "done" | "error" = error
    ? "error"
    : running
      ? "running"
      : result
        ? "done"
        : "idle";

  const statusLabel: Record<typeof status, string> = {
    idle: t("status.idle"),
    running: t("status.running"),
    done: t("status.done"),
    error: t("status.error"),
  };

  return (
    <main className="app">
      <header className="app-header">
        <img src="/icon.svg" alt="" className="app-logo" aria-hidden="true" />
        <span className={`status-dot status-dot--${status}`} aria-hidden="true" />
        <span className="app-mark">Subtitles</span>
        <span className="app-status">{statusLabel[status]}</span>
        <button
          className="btn-icon"
          onClick={() => setShowSettings((v) => !v)}
          type="button"
          aria-label={t("header.settings")}
        >
          <svg viewBox="0 0 24 24" fill="none" aria-hidden="true">
            <path
              d="M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6Z"
              stroke="currentColor"
              strokeWidth="1.6"
            />
            <path
              d="M19.4 13a7.4 7.4 0 0 0 0-2l1.9-1.5-2-3.4-2.2.9a7.6 7.6 0 0 0-1.7-1L15 3.6h-6l-.4 2.4a7.6 7.6 0 0 0-1.7 1l-2.2-.9-2 3.4L4.6 11a7.4 7.4 0 0 0 0 2l-1.9 1.5 2 3.4 2.2-.9a7.6 7.6 0 0 0 1.7 1l.4 2.4h6l.4-2.4a7.6 7.6 0 0 0 1.7-1l2.2.9 2-3.4L19.4 13Z"
              stroke="currentColor"
              strokeWidth="1.4"
              strokeLinejoin="round"
            />
          </svg>
        </button>
      </header>

      {showSettings && settings && (
        <div className="panel settings-panel">
          <div className="settings-row">
            <span className="settings-label">{t("settings.stt")}</span>
            <select
              className="settings-select"
              value={settings.stt_engine}
              onChange={(e) =>
                updateSettings({ stt_engine: e.currentTarget.value as SttEngineChoice })
              }
            >
              {STT_ENGINES.map((o) => (
                <option key={o.value} value={o.value}>
                  {t(o.labelKey)}
                </option>
              ))}
            </select>
          </div>
          {currentSttEngine?.keyName && (
            <ApiKeyField
              keyName={currentSttEngine.keyName}
              placeholder={currentSttEngine.keyPlaceholder ?? ""}
              t={t}
            />
          )}

          <div className="settings-row">
            <span className="settings-label">{t("settings.translation")}</span>
            <select
              className="settings-select"
              value={settings.translation_engine}
              onChange={(e) =>
                updateSettings({
                  translation_engine: e.currentTarget.value as TranslationEngineChoice,
                })
              }
            >
              {TRANSLATION_ENGINES.map((o) => (
                <option key={o.value} value={o.value}>
                  {t(o.labelKey)}
                </option>
              ))}
            </select>
          </div>
          {currentTranslationEngine?.keyName && (
            <ApiKeyField
              keyName={currentTranslationEngine.keyName}
              placeholder={currentTranslationEngine.keyPlaceholder ?? ""}
              t={t}
            />
          )}
          {currentTranslationEngine?.needsRegion && (
            <div className="settings-row">
              <span className="settings-label">{t("settings.azureRegion")}</span>
              <input
                className="settings-input settings-input-boxed"
                type="text"
                placeholder="westeurope"
                value={settings.azure_translator_region}
                onChange={(e) => updateSettings({ azure_translator_region: e.currentTarget.value })}
              />
            </div>
          )}

          <div className="settings-row">
            <span className="settings-label">{t("settings.language")}</span>
            <select
              className="settings-select"
              value={settings.language}
              onChange={(e) => updateSettings({ language: e.currentTarget.value as Lang })}
            >
              <option value="en">English</option>
              <option value="fr">Français</option>
            </select>
          </div>

          <div className="settings-row">
            <span className="settings-label">{t("settings.version")}</span>
            <span className="settings-value">{version ? `v${version}` : "…"}</span>
          </div>

          <div className="settings-row">
            <span className="settings-label">{t("settings.updates")}</span>
            <div className="cmd-row">
              <button
                className="btn btn-ghost"
                onClick={checkForUpdates}
                disabled={updateStatus === "checking" || updateStatus === "installing"}
                type="button"
              >
                {t("update.check")}
              </button>
              {updateStatus === "checking" && (
                <span className="update-status">{t("update.checking")}</span>
              )}
              {updateStatus === "up_to_date" && (
                <span className="update-status">{t("update.upToDate")}</span>
              )}
              {updateStatus === "installing" && (
                <span className="update-status">{t("update.installing")}</span>
              )}
              {updateStatus === "error" && (
                <span className="update-status update-status--error">
                  {t("update.failed", { error: updateError ?? "" })}
                </span>
              )}
            </div>
          </div>
        </div>
      )}

      <div className="app-body">
        {!inputPath && (
          <button className="dropzone" onClick={pickFile} type="button">
            <svg className="dropzone-icon" viewBox="0 0 48 48" fill="none" aria-hidden="true">
              <path
                d="M24 6v22m0 0-8-8m8 8 8-8M10 34v4a4 4 0 0 0 4 4h20a4 4 0 0 0 4-4v-4"
                stroke="currentColor"
                strokeWidth="2.4"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
            </svg>
            <span className="dropzone-title">{t("drop.title")}</span>
            <span className="dropzone-hint">mp4 · mov · mkv · mp3 · wav · m4a · flac</span>
          </button>
        )}

        {inputPath && (
          <div className="file-chip">
            <svg className="file-chip-icon" viewBox="0 0 24 24" fill="none" aria-hidden="true">
              <path
                d="M4 4.5A1.5 1.5 0 0 1 5.5 3H13l5 5v12.5A1.5 1.5 0 0 1 16.5 22h-11A1.5 1.5 0 0 1 4 20.5v-16Z"
                stroke="currentColor"
                strokeWidth="1.6"
                strokeLinejoin="round"
              />
              <path d="M13 3v5h5" stroke="currentColor" strokeWidth="1.6" strokeLinejoin="round" />
            </svg>
            <span className="file-chip-name" title={inputPath}>
              {fileName(inputPath)}
            </span>
            <button className="btn-link" onClick={pickFile} disabled={running} type="button">
              {t("file.change")}
            </button>
          </div>
        )}

        {inputPath && !result && (
          <button className="btn btn-primary" onClick={generate} disabled={running} type="button">
            {running ? t("generate.running") : t("generate.run")}
          </button>
        )}

        {running && !isTranslateStage && (
          <div className="panel progress-panel">
            <ol className="stepper">
              {transcribeStages.map((stage, i) => (
                <li
                  key={stage.key}
                  className={
                    "step" +
                    (i < stageIndex ? " step--done" : i === stageIndex ? " step--active" : "")
                  }
                >
                  <span className="step-marker">{i < stageIndex ? "✓" : i + 1}</span>
                  <span className="step-label">{t(stage.labelKey)}</span>
                </li>
              ))}
            </ol>
            <div className="progress-track">
              <div
                className="progress-fill"
                style={{ width: `${Math.round((progress?.fraction ?? 0) * 100)}%` }}
              />
            </div>
          </div>
        )}

        {error && (
          <div className="panel error-panel">
            <strong>{t("generate.failed")}</strong>
            <span>{error}</span>
          </div>
        )}

        {result && (
          <div className="panel result-panel">
            <div className="result-summary">
              <span className="pill">{result.language.toUpperCase()}</span>
              <span className="result-meta">{plural("result.lines", cues.length)}</span>
              <span className="result-path" title={result.srt_path}>
                {result.srt_path}
              </span>
              <button className="btn btn-ghost" onClick={saveAs} type="button">
                {t("result.saveAs")}
              </button>
            </div>

            <CueList cues={cues} />
          </div>
        )}

        {result && settings?.translation_engine !== "none" && (
          <div className="panel translate-panel">
            <div className="translate-controls">
              <select
                className="settings-select"
                value={sourceLang}
                onChange={(e) => setSourceLang(e.currentTarget.value)}
              >
                <option value="auto">
                  {settings?.translation_engine === "local"
                    ? t("translate.autoDetected")
                    : t("translate.autoDetect")}
                </option>
                {languages.map((l) => (
                  <option key={l.code} value={l.code}>
                    {l.name}
                  </option>
                ))}
              </select>
              <span className="translate-arrow">→</span>
              <select
                className="settings-select"
                value={targetLang}
                onChange={(e) => setTargetLang(e.currentTarget.value)}
              >
                {languages.map((l) => (
                  <option key={l.code} value={l.code}>
                    {l.name}
                  </option>
                ))}
              </select>
              <button
                className="btn btn-primary translate-btn"
                onClick={runTranslation}
                disabled={translating}
                type="button"
              >
                {translating ? t("translate.running") : t("translate.run")}
              </button>
            </div>

            {translating && isTranslateStage && (
              <div className="progress-track">
                <div
                  className="progress-fill"
                  style={{ width: `${Math.round((progress?.fraction ?? 0) * 100)}%` }}
                />
              </div>
            )}

            {translationError && (
              <p className="error-inline">
                {t("translate.failed", { error: translationError })}
              </p>
            )}

            {translation && (
              <>
                <div className="result-summary">
                  <span className="pill">{targetLang.toUpperCase()}</span>
                  <span className="result-path" title={translation.srt_path}>
                    {translation.srt_path}
                  </span>
                  <button className="btn btn-ghost" onClick={saveTranslationAs} type="button">
                    {t("result.saveAs")}
                  </button>
                </div>
                <CueList cues={translatedCues} />
              </>
            )}
          </div>
        )}
      </div>
    </main>
  );
}

export default App;
