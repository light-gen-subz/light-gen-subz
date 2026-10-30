import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { getPlural, getT, type Lang } from "./i18n";
import { SuiteRoot } from "./suite/SuiteRoot";
import { Home } from "./Home";
import { addRecent, clearRecent, loadRecent, removeRecent, type RecentEntry } from "./recent";
import type { MenuSpec } from "./suite/menu";
import { useSuiteMenu } from "./suite/useSuiteMenu";
import { changelogs } from "./suite/changelogs";
import "./App.css";

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
  /** The language the translation is in (not the one currently picked as target). */
  language: string;
};

/** What a failed command rejects with (see `CommandError` on the Rust side). */
type AudioTrack = {
  position: number;
  language: string | null;
  title: string | null;
  codec: string | null;
  channels: number | null;
};

type EditedSubtitles = { srt_content: string };

type BackendError = { code: string | null; message: string };

export type Cue = {
  index: number;
  start: string;
  end: string;
  text: string;
};

type SttEngineChoice = "local" | "groq" | "open_ai" | "deepgram" | "assembly_ai";
type TranslationEngineChoice = "local" | "deep_l" | "open_ai" | "google" | "azure";

type Settings = {
  stt_engine: SttEngineChoice;
  translation_engine: TranslationEngineChoice;
  azure_translator_region: string;
  language: Lang;
  whisper_model: string;
  stt_language: string;
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
  { value: "deepgram", labelKey: "stt.deepgram", keyName: "deepgram_api_key" },
  {
    value: "assembly_ai",
    labelKey: "stt.assemblyai",
    keyName: "assemblyai_api_key",
  },
];

const TRANSLATION_ENGINES: EngineOption<TranslationEngineChoice>[] = [
  { value: "local", labelKey: "tr.local" },
  {
    value: "deep_l",
    labelKey: "tr.deepl",
    keyName: "deepl_api_key",
    keyPlaceholder: "xxxxxxxx-...:fx",
  },
  { value: "open_ai", labelKey: "tr.openai", keyName: "openai_api_key", keyPlaceholder: "sk-..." },
  {
    value: "google",
    labelKey: "tr.google",
    keyName: "google_translate_api_key",
  },
  {
    value: "azure",
    labelKey: "tr.azure",
    keyName: "azure_translator_key",
    needsRegion: true,
  },
];

const WHISPER_MODELS = ["base", "small", "medium"];

/** Same list as the backend's file picker; a dropped file is checked against it. */
export const MEDIA_EXTENSIONS = [
  "mp4",
  "mkv",
  "mov",
  "avi",
  "webm",
  "mp3",
  "wav",
  "m4a",
  "flac",
  "ogg",
];

// The backend extracts the audio first, then fetches the model, so that is the order shown.
const TRANSCRIBE_STAGES: { key: Stage; labelKey: string }[] = [
  { key: "extract_audio", labelKey: "stage.audio" },
  { key: "download_model", labelKey: "stage.model" },
  { key: "transcribe", labelKey: "stage.transcript" },
  { key: "write_subtitles", labelKey: "stage.subtitles" },
];

export function fileName(path: string): string {
  return path.split(/[\\/]/).pop() ?? path;
}

export function parseSrt(content: string): Cue[] {
  return content
    .trim()
    .split(/\r?\n(?:[ \t]*\r?\n)+/)
    .map((block) => {
      const lines = block.split(/\r?\n/);
      // The sequence number is optional: a block may start with the timing line.
      const numbered = !(lines[0] ?? "").includes("-->");
      const [index, timing, ...textLines] = numbered ? lines : ["0", ...lines];
      const [start, end] = (timing ?? "").split(" --> ");
      return {
        index: Number(index) || 0,
        start: (start ?? "").trim(),
        end: (end ?? "").trim(),
        text: textLines
          .map((line) => line.trim())
          .filter(Boolean)
          .join("\n"),
      };
    })
    .filter((cue) => cue.text.length > 0);
}

export function hasMediaExtension(path: string): boolean {
  const ext = fileName(path).split(".").pop()?.toLowerCase() ?? "";
  return MEDIA_EXTENSIONS.includes(ext);
}

/** A readable message for whatever a command rejected with. */
export function formatError(e: unknown, t: ReturnType<typeof getT>): string {
  if (e && typeof e === "object" && "message" in e) {
    const { code, message } = e as BackendError;
    if (code) {
      const known = t(`error.${code}`, { detail: message });
      if (known !== `error.${code}`) return known;
    }
    return String(message);
  }
  return String(e);
}

function isCancelled(e: unknown): boolean {
  return typeof e === "object" && e !== null && (e as BackendError).code === "cancelled";
}

/** The language's name in the interface language, falling back to the backend's. */
function languageName(info: LanguageInfo, lang: Lang): string {
  try {
    const code = info.code === "zh" ? "zh-Hans" : info.code;
    return new Intl.DisplayNames([lang], { type: "language" }).of(code) ?? info.name;
  } catch {
    return info.name;
  }
}

/** A language name from an ISO code of two or three letters (`fra`, `eng`, `fr`). */
function codeName(code: string, lang: Lang): string {
  try {
    return new Intl.DisplayNames([lang], { type: "language" }).of(code) ?? code;
  } catch {
    return code;
  }
}

/** `83` → `1:23`, `3725` → `1:02:05`. */
export function formatDuration(totalSeconds: number): string {
  const secs = Math.max(0, Math.round(totalSeconds));
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  const s = secs % 60;
  const pad = (n: number) => String(n).padStart(2, "0");
  return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`;
}

/** Time spent in the current stage, and what is left once the progress allows a guess. */
function Elapsed({ fraction, t }: { fraction: number; t: ReturnType<typeof getT> }) {
  const [elapsed, setElapsed] = useState(0);
  useEffect(() => {
    const start = Date.now();
    const id = setInterval(() => setElapsed((Date.now() - start) / 1000), 1000);
    return () => clearInterval(id);
  }, []);
  const remaining =
    fraction >= 0.03 && fraction < 0.995 && elapsed >= 5
      ? (elapsed * (1 - fraction)) / fraction
      : null;
  return (
    <p className="hint" aria-live="off">
      {t("progress.elapsed", { time: formatDuration(elapsed) })}
      {remaining !== null && ` · ${t("progress.remaining", { time: formatDuration(remaining) })}`}
    </p>
  );
}

const CueList = memo(function CueList({ cues }: { cues: Cue[] }) {
  return (
    <ol className="cue-list">
      {cues.map((cue, position) => (
        <li className="cue" key={position}>
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
});

function RegenerateIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <path
        d="M20 11a8 8 0 0 0-14.6-4.5M4 13a8 8 0 0 0 14.6 4.5M5 3v4h4M19 21v-4h-4"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

function EditIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <path
        d="M4 20h4L19 9a2.8 2.8 0 0 0-4-4L4 16v4ZM13.5 6.5l4 4"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

type ExportFormat = "srt" | "vtt" | "ass";
const EXPORT_FORMATS: { value: ExportFormat; label: string }[] = [
  { value: "srt", label: "SubRip (.srt)" },
  { value: "vtt", label: "WebVTT (.vtt)" },
  { value: "ass", label: "Advanced SubStation (.ass)" },
];

/** The export window: which subtitles (when there is a translation) and which format. */
function ExportModal({
  t,
  hasTranslation,
  onExport,
  onClose,
}: {
  t: ReturnType<typeof getT>;
  hasTranslation: boolean;
  onExport: (targets: ("source" | "translation")[], format: ExportFormat) => void;
  onClose: () => void;
}) {
  const [format, setFormat] = useState<ExportFormat>("srt");
  const [targets, setTargets] = useState<("source" | "translation")[]>(["source"]);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);
  return (
    <div className="cl-backdrop">
      <div className="cl-scrim" aria-hidden="true" onClick={onClose} />
      <div
        className="cl-card export-card"
        role="dialog"
        aria-modal="true"
        aria-label={t("export.title")}
      >
        <div className="cl-title">{t("export.title")}</div>
        {hasTranslation && (
          <fieldset className="export-group">
            <legend>{t("export.subtitles")}</legend>
            {(["source", "translation"] as const).map((value) => (
              <label key={value} className="export-option">
                <input
                  type="checkbox"
                  checked={targets.includes(value)}
                  onChange={(e) => {
                    const { checked } = e.currentTarget;
                    setTargets((current) =>
                      checked ? [...current, value] : current.filter((v) => v !== value),
                    );
                  }}
                />
                {t(value === "source" ? "export.original" : "export.translation")}
              </label>
            ))}
          </fieldset>
        )}
        <fieldset className="export-group">
          <legend>{t("export.format")}</legend>
          {EXPORT_FORMATS.map((f) => (
            <label key={f.value} className="export-option">
              <input
                type="radio"
                name="export-format"
                checked={format === f.value}
                onChange={() => setFormat(f.value)}
              />
              {f.label}
            </label>
          ))}
        </fieldset>
        <div className="export-actions">
          <button type="button" className="sb-ghost" onClick={onClose}>
            {t("export.cancel")}
          </button>
          <button
            type="button"
            className="sb-primary"
            disabled={targets.length === 0}
            onClick={() => {
              onExport(targets, format);
              onClose();
            }}
          >
            {t("export.confirm")}
          </button>
        </div>
      </div>
    </div>
  );
}

const CueEditRow = memo(function CueEditRow({
  index,
  cue,
  value,
  label,
  onChange,
}: {
  index: number;
  cue: Cue;
  value: string;
  label: string;
  onChange: (index: number, value: string) => void;
}) {
  return (
    <li className="cue">
      <span className="cue-index">{cue.index}</span>
      <div className="cue-body">
        <span className="cue-time">
          {cue.start} <span className="cue-arrow">→</span> {cue.end}
        </span>
        <textarea
          className="cue-edit"
          rows={3}
          aria-label={label}
          value={value}
          onChange={(e) => onChange(index, e.currentTarget.value)}
        />
      </div>
    </li>
  );
});

function CueEditor({
  cues,
  t,
  onApply,
  onCancel,
}: {
  cues: Cue[];
  t: ReturnType<typeof getT>;
  onApply: (texts: string[]) => void;
  onCancel: () => void;
}) {
  const [texts, setTexts] = useState(() => cues.map((c) => c.text));
  const [confirmRemoval, setConfirmRemoval] = useState(false);
  const changed = texts.some((text, i) => text !== cues[i].text);
  const emptied = texts.filter((text) => !text.trim()).length;
  // One row re-renders per keystroke, not the whole list.
  const change = useCallback((index: number, value: string) => {
    setConfirmRemoval(false);
    setTexts((previous) => previous.map((old, j) => (j === index ? value : old)));
  }, []);
  return (
    <>
      <ol className="cue-list">
        {cues.map((cue, i) => (
          <CueEditRow
            key={i}
            index={i}
            cue={cue}
            value={texts[i]}
            label={t("edit.cue", { index: cue.index })}
            onChange={change}
          />
        ))}
      </ol>
      {emptied > 0 && (
        <p className="hint" role="alert">
          {t("edit.emptyWarning", { count: emptied })}
        </p>
      )}
      <div className="action-row">
        <button
          className="btn btn-primary"
          type="button"
          disabled={!changed}
          onClick={() => {
            if (emptied > 0 && !confirmRemoval) setConfirmRemoval(true);
            else onApply(texts);
          }}
        >
          {confirmRemoval ? t("edit.confirmRemove") : t("edit.apply")}
        </button>
        <button className="btn btn-ghost" type="button" onClick={onCancel}>
          {t("edit.discard")}
        </button>
      </div>
    </>
  );
}

function trackLabel(track: AudioTrack, lang: Lang): string {
  const details = [track.codec, track.channels ? `${track.channels}ch` : null].filter(Boolean);
  const language = track.language ? codeName(track.language, lang) : null;
  const name = [language, track.title].filter(Boolean).join(" · ");
  return `#${track.position + 1}${name ? ` — ${name}` : ""}${details.length ? ` (${details.join(", ")})` : ""}`;
}

function ProgressBar({ fraction, label }: { fraction: number; label: string }) {
  const percent = Math.round(fraction * 100);
  return (
    <div
      className="progress-track"
      role="progressbar"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={percent}
    >
      <div className="progress-fill" style={{ width: `${percent}%` }} />
    </div>
  );
}

function ApiKeyField({
  keyName,
  placeholder,
  t,
  onChange,
}: {
  keyName: string;
  placeholder: string;
  t: ReturnType<typeof getT>;
  onChange: () => void;
}) {
  const [value, setValue] = useState("");
  const [hasKey, setHasKey] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    invoke<boolean>("has_api_key", { keyName })
      .then(setHasKey)
      .catch(() => {}); // unknown: shown as "no key saved"
  }, [keyName]);

  async function saveKey() {
    if (!value.trim()) return;
    try {
      await invoke("set_api_key", { keyName, value: value.trim() });
      setHasKey(true);
      setValue("");
      setError(null);
      onChange();
    } catch (e) {
      setError(formatError(e, t));
    }
  }

  async function removeKey() {
    try {
      await invoke("delete_api_key", { keyName });
      setHasKey(false);
      setError(null);
      onChange();
    } catch (e) {
      setError(formatError(e, t));
    }
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
          aria-label={t("settings.apiKey")}
          placeholder={placeholder || t("settings.apiKey")}
          value={value}
          onChange={(e) => setValue(e.currentTarget.value)}
        />
        <button className="btn btn-ghost" onClick={saveKey} type="button">
          {t("settings.save")}
        </button>
        {hasKey && (
          <button className="btn btn-ghost" onClick={removeKey} type="button">
            {t("settings.remove")}
          </button>
        )}
      </div>
      {error && (
        <p className="error-inline" role="alert">
          {t("settings.keyError", { error })}
        </p>
      )}
    </div>
  );
}

function AppBody({ onLangChange }: { onLangChange: (lang: Lang) => void }) {
  const [inputPath, setInputPath] = useState<string | null>(null);
  const [recent, setRecent] = useState<RecentEntry[]>(loadRecent);
  const [progress, setProgress] = useState<PipelineProgress | null>(null);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<PipelineOutput | null>(null);
  const [dragging, setDragging] = useState(false);
  const [audioTracks, setAudioTracks] = useState<AudioTrack[]>([]);
  const [audioTrack, setAudioTrack] = useState<number | null>(null);
  const [editing, setEditing] = useState<"source" | "translation" | null>(null);
  const [editError, setEditError] = useState<{
    target: "source" | "translation";
    message: string;
  } | null>(null);

  const [settings, setSettings] = useState<Settings | null>(null);
  const [languages, setLanguages] = useState<LanguageInfo[]>([]);
  const [showSettings, setShowSettings] = useState(false);
  const [showExport, setShowExport] = useState(false);
  const [region, setRegion] = useState("");
  const [keyVersion, setKeyVersion] = useState(0);
  // Whether each key was found, by key name; a key not looked up yet counts as present.
  const [keyPresence, setKeyPresence] = useState<Record<string, boolean>>({});

  const [sourceLang, setSourceLang] = useState("auto");
  const [detectedLang, setDetectedLang] = useState<string | null>(null);
  const [targetLang, setTargetLang] = useState("en");
  const [translating, setTranslating] = useState(false);
  const [translationError, setTranslationError] = useState<string | null>(null);
  const [translation, setTranslation] = useState<TranslationOutput | null>(null);
  const [cancelling, setCancelling] = useState(false);
  const [settingsError, setSettingsError] = useState<string | null>(null);
  const [notice, setNotice] = useState<{
    target: "source" | "translation";
    failed: boolean;
    text: string;
  } | null>(null);

  // A run that was superseded (another file picked meanwhile) must not write its result.
  const runId = useRef(0);
  const busy = useRef(false);
  // Settings are saved one after the other, in the order they were changed: two saves
  // in flight could reach the disk reversed and keep the older value.
  const settingsRef = useRef<Settings | null>(null);
  const saveChain = useRef<Promise<void>>(Promise.resolve());
  useEffect(() => {
    busy.current = running || translating;
  }, [running, translating]);

  useEffect(() => {
    const unlisten = listen<PipelineProgress>("pipeline-progress", (event) => {
      setProgress(event.payload);
    });
    return () => {
      unlisten.then((f) => f()).catch(() => {});
    };
  }, []);

  useEffect(() => {
    invoke<Settings>("get_settings")
      .then((loaded) => {
        settingsRef.current = loaded;
        setSettings(loaded);
        setRegion(loaded.azure_translator_region);
      })
      .catch(() => {});
    invoke<LanguageInfo[]>("list_languages")
      .then(setLanguages)
      .catch(() => {});
  }, []);

  const lang: Lang = settings?.language ?? "en";
  const t = useMemo(() => getT(lang), [lang]);
  const plural = useMemo(() => getPlural(lang), [lang]);

  useEffect(() => {
    document.documentElement.lang = lang;
    onLangChange(lang);
  }, [lang, onLangChange]);

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

  // Warn before a run, not after it failed for lack of a key.
  const sttKeyName = currentSttEngine?.keyName;
  const translationKeyName = currentTranslationEngine?.keyName;
  useEffect(() => {
    for (const keyName of [sttKeyName, translationKeyName]) {
      if (!keyName) continue;
      invoke<boolean>("has_api_key", { keyName })
        .then((has) => setKeyPresence((previous) => ({ ...previous, [keyName]: has })))
        .catch(() => {});
    }
  }, [sttKeyName, translationKeyName, keyVersion]);
  const sttKeyMissing = sttKeyName !== undefined && keyPresence[sttKeyName] === false;
  const translationKeyMissing =
    translationKeyName !== undefined && keyPresence[translationKeyName] === false;

  const updateSettings = useCallback(
    (patch: Partial<Settings>) => {
      const current = settingsRef.current;
      if (!current) return;
      const next = { ...current, ...patch };
      settingsRef.current = next;
      setSettings(next);
      saveChain.current = saveChain.current
        .then(() => invoke("set_settings", { settings: next }))
        .then(() => setSettingsError(null))
        .catch((e) => setSettingsError(formatError(e, t)));
    },
    [t],
  );

  // One write when the user stops typing, not one per keystroke.
  useEffect(() => {
    if (!settings || region === settings.azure_translator_region) return;
    const timer = setTimeout(() => updateSettings({ azure_translator_region: region }), 300);
    return () => clearTimeout(timer);
  }, [region, settings, updateSettings]);

  function selectFile(path: string) {
    if (!hasMediaExtension(path)) {
      setError(t("file.unsupported", { name: fileName(path) }));
      return;
    }
    const id = ++runId.current;
    setRecent((list) => addRecent(list, path));
    setInputPath(path);
    setResult(null);
    setError(null);
    setTranslation(null);
    setTranslationError(null);
    setDetectedLang(null);
    setSourceLang("auto");
    setEditing(null);
    setEditError(null);
    setNotice(null);
    setAudioTrack(null);
    setAudioTracks([]);
    invoke<AudioTrack[]>("list_audio_tracks", { path })
      .then((tracks) => {
        if (runId.current === id) setAudioTracks(tracks);
      })
      .catch(() => {});
  }

  async function pickFile() {
    try {
      const path = await invoke<string | null>("pick_file", {
        filterName: t("dialog.mediaFilter"),
      });
      if (path) selectFile(path);
    } catch (e) {
      setError(formatError(e, t));
    }
  }

  // Dropping a file on the window selects it, like the picker does.
  const selectDropped = useRef(selectFile);
  useEffect(() => {
    selectDropped.current = selectFile;
  });
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let disposed = false;
    try {
      getCurrentWebview()
        .onDragDropEvent((event) => {
          const payload = event.payload;
          if (payload.type === "enter" || payload.type === "over") {
            setDragging(!busy.current);
          } else if (payload.type === "drop") {
            setDragging(false);
            if (!busy.current && payload.paths.length > 0) selectDropped.current(payload.paths[0]);
          } else {
            setDragging(false);
          }
        })
        .then((f) => (disposed ? f() : (unlisten = f)))
        .catch(() => {});
    } catch {
      // Not running inside Tauri (e.g. a plain browser): drag and drop is simply absent.
    }
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  async function generate() {
    if (!inputPath) return;
    const id = ++runId.current;
    setRunning(true);
    setCancelling(false);
    setNotice(null);
    setError(null);
    setResult(null);
    setTranslation(null);
    setTranslationError(null);
    setProgress(null);
    try {
      const output = await invoke<PipelineOutput>("run_pipeline", { inputPath, audioTrack });
      if (id !== runId.current) return;
      setResult(output);
      const detected = output.language.slice(0, 3).toLowerCase();
      const known = languages.some((l) => l.code === detected) ? detected : null;
      setDetectedLang(known);
      setSourceLang(known ?? "auto");
      // Translating into the language just spoken makes no sense: start on another one.
      if (known && known === targetLang) setTargetLang(known === "en" ? "fr" : "en");
    } catch (e) {
      if (id === runId.current && !isCancelled(e)) setError(formatError(e, t));
    } finally {
      if (id === runId.current) {
        setRunning(false);
        setCancelling(false);
      }
    }
  }

  // The request in flight is not interrupted at once: say that the stop was heard.
  function cancel() {
    setCancelling(true);
    invoke("cancel_pipeline").catch(() => setCancelling(false));
  }

  async function applyEdits(target: "source" | "translation", texts: string[]) {
    const output = target === "source" ? result : translation;
    if (!output) return;
    try {
      const edited = await invoke<EditedSubtitles>("update_subtitles", {
        srtContent: output.srt_content,
        texts,
      });
      const next = { ...output, srt_content: edited.srt_content };
      if (target === "source") setResult(next);
      else setTranslation(next);
      setEditing(null);
      setEditError(null);
    } catch (e) {
      setEditError({ target, message: t("edit.failed", { error: formatError(e, t) }) });
    }
  }

  /** One save dialog, whatever is exported: the first target is named by the user, the
   * other is saved next to it with its own suffix (`clip.srt`, `clip.fr.srt`). */
  async function exportSubtitles(
    targets: ("source" | "translation")[],
    format: "srt" | "vtt" | "ass",
  ) {
    const outputs = targets.flatMap((target) => {
      const output = target === "source" ? result : translation;
      return output ? [{ target, output }] : [];
    });
    if (outputs.length === 0) return;
    const [first, ...others] = outputs;
    try {
      const saved = await invoke<string[] | null>("save_subtitle_as", {
        defaultPath: first.output.srt_path,
        filterName: t(
          format === "srt"
            ? "dialog.srtFilter"
            : format === "vtt"
              ? "dialog.vttFilter"
              : "dialog.assFilter",
        ),
        format,
        srtContent: first.output.srt_content,
        extras: others.map(({ output }) => ({
          // Fields of a nested struct are not renamed by Tauri: snake_case, as Rust reads them.
          srt_path: output.srt_path,
          srt_content: output.srt_content,
        })),
      });
      // `null`: the dialog was dismissed, nothing to report.
      if (saved) {
        setNotice({
          target: first.target,
          failed: false,
          text: t("result.saved", { path: saved.join(", ") }),
        });
      }
    } catch (e) {
      setNotice({
        target: first.target,
        failed: true,
        text: t("result.saveFailed", { error: formatError(e, t) }),
      });
    }
  }

  // Left on "Auto", the language found while transcribing is used: the local translator
  // cannot detect one, and the cloud ones guess line by line, wrongly on short lines.
  const effectiveSource = sourceLang !== "auto" ? sourceLang : detectedLang;
  const sameLanguage = effectiveSource === targetLang;

  async function runTranslation() {
    if (!result) return;
    const id = ++runId.current;
    setTranslating(true);
    setCancelling(false);
    setNotice(null);
    setTranslationError(null);
    setTranslation(null);
    // A new translation replaces the one that may be open in the editor.
    setEditing((current) => (current === "translation" ? null : current));
    setEditError(null);
    setProgress(null);
    try {
      const output = await invoke<TranslationOutput>("translate_subtitles", {
        srtPath: result.srt_path,
        srtContent: result.srt_content,
        sourceLang: effectiveSource,
        targetLang,
      });
      if (id === runId.current) setTranslation(output);
    } catch (e) {
      if (id === runId.current && !isCancelled(e)) setTranslationError(formatError(e, t));
    } finally {
      if (id === runId.current) {
        setTranslating(false);
        setCancelling(false);
      }
    }
  }

  const recentSpecs = useMemo<MenuSpec[]>(
    () => [
      ...recent.map((item): MenuSpec => ({
        kind: "item",
        text: item.name,
        action: () => selectFile(item.path),
      })),
      ...(recent.length ? ([{ kind: "separator" }] as MenuSpec[]) : []),
      { kind: "item", text: t("recent.clear"), action: () => setRecent(clearRecent()) },
    ],
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [recent, t],
  );
  const file = useMemo<MenuSpec[]>(
    () => [
      {
        kind: "item",
        text: t("menu.open"),
        accelerator: "CmdOrCtrl+O",
        action: () => void pickFile(),
      },
      {
        kind: "submenu",
        text: t("menu.openRecent"),
        enabled: recent.length > 0,
        items: recentSpecs,
      },
      { kind: "separator" },
      {
        kind: "item",
        text: t("menu.settings"),
        accelerator: "CmdOrCtrl+,",
        action: () => setShowSettings((v) => !v),
      },
    ],
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [t, recent, recentSpecs],
  );
  const onLanguage = useCallback((l: Lang) => updateSettings({ language: l }), [updateSettings]);
  useSuiteMenu({ t, lang, appName: "LightGenSubz", onLanguage, file });

  useEffect(() => {
    if (!showSettings) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setShowSettings(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [showSettings]);

  useEffect(() => {
    if (!showSettings) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setShowSettings(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [showSettings]);

  // The same shortcuts as the menu entries, for when the native menu is not there.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.ctrlKey || e.metaKey)) return;
      if (e.key === ",") setShowSettings((v) => !v);
      else if (e.key.toLowerCase() === "o") void pickFile();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const isCloudStt = settings !== null && settings.stt_engine !== "local";

  return (
    <main className="app">
      {inputPath && (
        <header className="app-header">
          <div className="file-badge" title={inputPath}>
            <span className="file-badge-name">{fileName(inputPath)}</span>
            <span className="file-badge-meta">
              {[
                fileName(inputPath).split(".").pop()?.toUpperCase(),
                result?.language.toUpperCase(),
                result ? plural("result.lines", cues.length) : null,
              ]
                .filter(Boolean)
                .join(" · ")}
            </span>
          </div>
          <div className="top-bar-actions">
            <button
              type="button"
              className="sb-primary"
              disabled={!result}
              onClick={() => setShowExport(true)}
            >
              {t("export.button")}
            </button>
          </div>
        </header>
      )}

      {showExport && result && (
        <ExportModal
          t={t}
          hasTranslation={!!translation}
          onExport={(targets, format) => void exportSubtitles(targets, format)}
          onClose={() => setShowExport(false)}
        />
      )}

      {showSettings && settings && (
        <>
          <div
            className="sheet-overlay"
            aria-hidden="true"
            onClick={() => setShowSettings(false)}
          />
          <aside className="sheet" role="dialog" aria-modal="true" aria-label={t("settings.title")}>
            <div className="sheet-header">
              <span className="sheet-title">{t("settings.title")}</span>
              <button
                type="button"
                className="sheet-close"
                onClick={() => setShowSettings(false)}
                aria-label={t("cl.close")}
              >
                <svg viewBox="0 0 24 24" fill="none" aria-hidden="true">
                  <path
                    d="M18 6L6 18M6 6l12 12"
                    stroke="currentColor"
                    strokeWidth="2"
                    strokeLinecap="round"
                  />
                </svg>
              </button>
            </div>
            <div className="settings-panel">
              <div className="settings-row">
                <label className="settings-label" htmlFor="stt-engine">
                  {t("settings.stt")}
                </label>
                <select
                  id="stt-engine"
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
              {isCloudStt && <p className="hint">{t("settings.cloudNotice")}</p>}
              {currentSttEngine?.keyName && (
                <ApiKeyField
                  keyName={currentSttEngine.keyName}
                  placeholder={currentSttEngine.keyPlaceholder ?? ""}
                  t={t}
                  onChange={() => setKeyVersion((v) => v + 1)}
                />
              )}
              {settings.stt_engine === "local" && (
                <div className="settings-row">
                  <label className="settings-label" htmlFor="whisper-model">
                    {t("settings.model")}
                  </label>
                  <select
                    id="whisper-model"
                    className="settings-select"
                    value={settings.whisper_model}
                    onChange={(e) => updateSettings({ whisper_model: e.currentTarget.value })}
                  >
                    {WHISPER_MODELS.map((name) => (
                      <option key={name} value={name}>
                        {t(`model.${name}`)}
                      </option>
                    ))}
                  </select>
                </div>
              )}
              <div className="settings-row">
                <label className="settings-label" htmlFor="stt-language">
                  {t("settings.sttLanguage")}
                </label>
                <select
                  id="stt-language"
                  className="settings-select"
                  value={settings.stt_language}
                  onChange={(e) => updateSettings({ stt_language: e.currentTarget.value })}
                >
                  <option value="auto">{t("stt.langAuto")}</option>
                  {languages.map((l) => (
                    <option key={l.code} value={l.code}>
                      {languageName(l, lang)}
                    </option>
                  ))}
                </select>
              </div>

              <div className="settings-row">
                <label className="settings-label" htmlFor="translation-engine">
                  {t("settings.translation")}
                </label>
                <select
                  id="translation-engine"
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
                  onChange={() => setKeyVersion((v) => v + 1)}
                />
              )}
              {currentTranslationEngine?.needsRegion && (
                <div className="settings-row">
                  <label className="settings-label" htmlFor="azure-region">
                    {t("settings.azureRegion")}
                  </label>
                  <input
                    id="azure-region"
                    className="settings-input settings-input-boxed"
                    type="text"
                    placeholder="westeurope"
                    value={region}
                    onChange={(e) => setRegion(e.currentTarget.value)}
                  />
                </div>
              )}

              {settingsError && (
                <p className="error-inline" role="alert">
                  {t("settings.saveFailed", { error: settingsError })}
                </p>
              )}
            </div>
          </aside>
        </>
      )}

      <div className="app-body">
        {!inputPath && (
          <Home
            t={t}
            dragging={dragging}
            formats={MEDIA_EXTENSIONS}
            recent={recent}
            onBrowse={pickFile}
            onOpenRecent={selectFile}
            onRemoveRecent={(path) => setRecent((list) => removeRecent(list, path))}
            onClearRecent={() => setRecent(clearRecent())}
          />
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
            <button
              type="button"
              className="icon-btn"
              title={t("generate.regenerate")}
              aria-label={t("generate.regenerate")}
              disabled={!result || running || translating}
              onClick={() => {
                setResult(null);
                setTranslation(null);
              }}
            >
              <RegenerateIcon />
            </button>
            <button
              className="btn-link"
              onClick={pickFile}
              disabled={running || translating}
              type="button"
            >
              {t("file.change")}
            </button>
          </div>
        )}

        {inputPath && !result && audioTracks.length > 1 && (
          <div className="settings-row">
            <label className="settings-label" htmlFor="audio-track">
              {t("audio.track")}
            </label>
            <select
              id="audio-track"
              className="settings-select"
              value={audioTrack ?? ""}
              disabled={running}
              onChange={(e) =>
                setAudioTrack(e.currentTarget.value === "" ? null : Number(e.currentTarget.value))
              }
            >
              <option value="">{t("audio.default")}</option>
              {audioTracks.map((track) => (
                <option key={track.position} value={track.position}>
                  {trackLabel(track, lang)}
                </option>
              ))}
            </select>
          </div>
        )}

        {inputPath && !result && (
          <div className="action-row">
            <button className="btn btn-primary" onClick={generate} disabled={running} type="button">
              {running ? t("generate.running") : t("generate.run")}
            </button>
            {running && (
              <button
                className="btn btn-ghost"
                onClick={cancel}
                disabled={cancelling}
                type="button"
              >
                {cancelling ? t("generate.cancelling") : t("generate.cancel")}
              </button>
            )}
          </div>
        )}

        {inputPath && !result && !running && sttKeyMissing && (
          <p className="hint">{t("generate.keyMissing")}</p>
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
            <ProgressBar fraction={progress?.fraction ?? 0} label={t("progress.label")} />
            <Elapsed key={progress?.stage ?? "start"} fraction={progress?.fraction ?? 0} t={t} />
          </div>
        )}

        {error && (
          <div className="panel error-panel" role="alert">
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
                {fileName(result.srt_path)}
              </span>
              <button
                type="button"
                className="icon-btn result-edit"
                title={t("edit.start")}
                aria-label={t("edit.start")}
                onClick={() => setEditing("source")}
                disabled={editing !== null || translating}
              >
                <EditIcon />
              </button>
            </div>
            {notice?.target === "source" && (
              <p
                className={notice.failed ? "error-inline" : "hint"}
                role={notice.failed ? "alert" : "status"}
              >
                {notice.text}
              </p>
            )}

            {editError?.target === "source" && (
              <p className="error-inline" role="alert">
                {editError.message}
              </p>
            )}
            {editing === "source" ? (
              <CueEditor
                cues={cues}
                t={t}
                onApply={(texts) => applyEdits("source", texts)}
                onCancel={() => setEditing(null)}
              />
            ) : (
              <CueList cues={cues} />
            )}
          </div>
        )}

        {result && (
          <div className="panel translate-panel">
            <div className="translate-controls">
              <select
                className="settings-select"
                aria-label={t("translate.source")}
                value={sourceLang}
                onChange={(e) => setSourceLang(e.currentTarget.value)}
              >
                <option value="auto">
                  {detectedLang ? t("translate.autoDetected") : t("translate.autoDetect")}
                </option>
                {languages.map((l) => (
                  <option key={l.code} value={l.code}>
                    {languageName(l, lang)}
                  </option>
                ))}
              </select>
              <span className="translate-arrow" aria-hidden="true">
                →
              </span>
              <select
                className="settings-select"
                aria-label={t("translate.target")}
                value={targetLang}
                onChange={(e) => setTargetLang(e.currentTarget.value)}
              >
                {languages.map((l) => (
                  <option key={l.code} value={l.code}>
                    {languageName(l, lang)}
                  </option>
                ))}
              </select>
              <button
                className="btn btn-primary translate-btn"
                onClick={runTranslation}
                disabled={translating || sameLanguage || editing === "source"}
                type="button"
              >
                {translating ? t("translate.running") : t("translate.run")}
              </button>
              {translating && (
                <button
                  className="btn btn-ghost"
                  onClick={cancel}
                  disabled={cancelling}
                  type="button"
                >
                  {cancelling ? t("generate.cancelling") : t("generate.cancel")}
                </button>
              )}
            </div>

            {sameLanguage && <p className="hint">{t("translate.same")}</p>}
            {!translating && translationKeyMissing && (
              <p className="hint">{t("translate.keyMissing")}</p>
            )}

            {translating && isTranslateStage && (
              <>
                <ProgressBar fraction={progress?.fraction ?? 0} label={t("progress.label")} />
                <Elapsed key={progress?.stage} fraction={progress?.fraction ?? 0} t={t} />
              </>
            )}

            {translationError && (
              <p className="error-inline" role="alert">
                {t("translate.failed", { error: translationError })}
              </p>
            )}

            {translation && (
              <>
                <div className="result-summary">
                  <span className="pill">{translation.language.toUpperCase()}</span>
                  <span className="result-path" title={translation.srt_path}>
                    {fileName(translation.srt_path)}
                  </span>
                  <button
                    type="button"
                    className="icon-btn result-edit"
                    title={t("edit.start")}
                    aria-label={t("edit.start")}
                    onClick={() => setEditing("translation")}
                    disabled={editing !== null}
                  >
                    <EditIcon />
                  </button>
                </div>
                {notice?.target === "translation" && (
                  <p
                    className={notice.failed ? "error-inline" : "hint"}
                    role={notice.failed ? "alert" : "status"}
                  >
                    {notice.text}
                  </p>
                )}
                {editError?.target === "translation" && (
                  <p className="error-inline" role="alert">
                    {editError.message}
                  </p>
                )}
                {editing === "translation" ? (
                  <CueEditor
                    cues={translatedCues}
                    t={t}
                    onApply={(texts) => applyEdits("translation", texts)}
                    onCancel={() => setEditing(null)}
                  />
                ) : (
                  <CueList cues={translatedCues} />
                )}
              </>
            )}
          </div>
        )}
      </div>
    </main>
  );
}

/** The app, inside what every app of the suite shares: update check and banner, What's new. */
function App() {
  const [lang, setLang] = useState<Lang>("en");
  const t = useMemo(() => getT(lang), [lang]);
  return (
    <SuiteRoot
      lang={lang}
      t={t}
      appId="light-gen-subz"
      appName="LightGenSubz"
      changelogs={changelogs}
      fetchNotes={(l, version) => invoke<string>("release_notes", { lang: l, version })}
    >
      <AppBody onLangChange={setLang} />
    </SuiteRoot>
  );
}

export default App;
