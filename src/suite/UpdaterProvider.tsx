import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { getVersion } from "@tauri-apps/api/app";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { entriesBetween, formatNotes, parseChangelog } from "./changelog";
import type { SuiteLang } from "./messages";

export type UpdatePhase = "idle" | "checking" | "available" | "downloading" | "installing";

export interface UpdateNotice {
  type: "success" | "error";
  /** A key of `suiteMessages`, or the text itself when the app has its own words. */
  key?: string;
  message?: string;
}

interface SuiteState {
  /** The installed version, empty until known. */
  version: string;
  phase: UpdatePhase;
  /** The update on offer, with what it brings in the user's language. */
  update: { version: string; notes: string } | null;
  /** 0-100, while downloading. */
  progress: number;
  notice: UpdateNotice | null;
  /** A user-driven check (says so when there is nothing new, or when it fails). */
  checkNow: () => Promise<void>;
  install: () => Promise<void>;
  /** "Later". */
  dismiss: () => void;
  clearNotice: () => void;
  changelogOpen: boolean;
  openChangelog: () => void;
  closeChangelog: () => void;
  aboutOpen: boolean;
  openAbout: () => void;
  closeAbout: () => void;
  /** A message of the app's own, shown as a toast. */
  notify: (type: UpdateNotice["type"], message: string) => void;
}

/** Without <SuiteRoot> (a component rendered on its own, in a test) the suite is inert. */
const inert: SuiteState = {
  version: "",
  phase: "idle",
  update: null,
  progress: 0,
  notice: null,
  checkNow: async () => {},
  install: async () => {},
  dismiss: () => {},
  clearNotice: () => {},
  changelogOpen: false,
  openChangelog: () => {},
  closeChangelog: () => {},
  aboutOpen: false,
  openAbout: () => {},
  closeAbout: () => {},
  notify: () => {},
};

const SuiteContext = createContext<SuiteState>(inert);

export function useSuite(): SuiteState {
  return useContext(SuiteContext);
}

interface Props {
  lang: SuiteLang;
  /** The app's changelog text at a release tag, in the given language ('' when unreachable). */
  fetchNotes: (lang: SuiteLang, version: string) => Promise<string>;
  /** Ask for an update a moment after start-up, without bothering the user if there is none. */
  checkOnStart?: boolean;
  /** Runs once the update is installed, before the restart: false cancels the restart (unsaved
   * work). */
  beforeRestart?: () => Promise<boolean>;
  children: ReactNode;
}

const START_DELAY_MS = 3000;

/** One update check at a time: the start-up check and a click on "Check for updates" can
 * overlap, and each would open its own banner. */
export function UpdaterProvider({
  lang,
  fetchNotes,
  checkOnStart = true,
  beforeRestart,
  children,
}: Props) {
  const [version, setVersion] = useState("");
  const [phase, setPhase] = useState<UpdatePhase>("idle");
  const [update, setUpdate] = useState<SuiteState["update"]>(null);
  const [progress, setProgress] = useState(0);
  const [notice, setNotice] = useState<UpdateNotice | null>(null);
  const [changelogOpen, setChangelogOpen] = useState(false);
  const [aboutOpen, setAboutOpen] = useState(false);

  const pending = useRef<Update | null>(null);
  const running = useRef<Promise<void> | null>(null);
  const installed = useRef("");
  const langRef = useRef(lang);
  useEffect(() => {
    langRef.current = lang;
  }, [lang]);

  useEffect(() => {
    void getVersion()
      .then((v) => {
        installed.current = v;
        setVersion(v);
      })
      .catch(() => {});
  }, []);

  const run = useCallback(
    async (manual: boolean) => {
      setPhase("checking");
      setNotice(null);
      try {
        const found = await check();
        if (!found) {
          setPhase("idle");
          if (manual) setNotice({ type: "success", key: "upd.upToDate" });
          return;
        }
        pending.current = found;
        let notes = "";
        try {
          const current = installed.current || (await getVersion());
          const text = await fetchNotes(langRef.current, found.version);
          notes = formatNotes(entriesBetween(parseChangelog(text), current, found.version));
        } catch {
          // The notes are a courtesy: the update is offered without them.
        }
        setUpdate({ version: found.version, notes });
        setPhase("available");
      } catch {
        setPhase("idle");
        if (manual) setNotice({ type: "error", key: "upd.failed" });
      }
    },
    [fetchNotes],
  );

  const checkNow = useCallback(() => {
    if (running.current) return running.current;
    running.current = run(true).finally(() => {
      running.current = null;
    });
    return running.current;
  }, [run]);

  useEffect(() => {
    if (!checkOnStart) return;
    const timer = setTimeout(() => {
      if (running.current) return;
      running.current = run(false).finally(() => {
        running.current = null;
      });
    }, START_DELAY_MS);
    return () => clearTimeout(timer);
  }, [checkOnStart, run]);

  const install = useCallback(async () => {
    const found = pending.current;
    if (!found) return;
    try {
      // The updater installs the package that matches how the app was installed, then the
      // app restarts.
      setPhase("downloading");
      setProgress(0);
      let downloaded = 0;
      let total = 0;
      await found.downloadAndInstall((event) => {
        if (event.event === "Started") {
          total = event.data.contentLength ?? 0;
        } else if (event.event === "Progress") {
          downloaded += event.data.chunkLength;
          if (total > 0) setProgress(Math.round((downloaded / total) * 100));
        } else if (event.event === "Finished") {
          setProgress(100);
        }
      });
      setPhase("installing");
      if (beforeRestart && !(await beforeRestart())) {
        setPhase("idle");
        setNotice({ type: "error", key: "upd.saveFirst" });
        return;
      }
      await relaunch();
    } catch {
      setPhase("idle");
      setNotice({ type: "error", key: "upd.failed" });
    }
  }, [beforeRestart]);

  const dismiss = useCallback(() => {
    setPhase("idle");
    setUpdate(null);
  }, []);
  const clearNotice = useCallback(() => setNotice(null), []);
  const openChangelog = useCallback(() => setChangelogOpen(true), []);
  const closeChangelog = useCallback(() => setChangelogOpen(false), []);
  const openAbout = useCallback(() => setAboutOpen(true), []);
  const closeAbout = useCallback(() => setAboutOpen(false), []);
  const notify = useCallback(
    (type: UpdateNotice["type"], message: string) => setNotice({ type, message }),
    [],
  );

  const value = useMemo<SuiteState>(
    () => ({
      version,
      phase,
      update,
      progress,
      notice,
      checkNow,
      install,
      dismiss,
      clearNotice,
      changelogOpen,
      openChangelog,
      closeChangelog,
      aboutOpen,
      openAbout,
      closeAbout,
      notify,
    }),
    [
      version,
      phase,
      update,
      progress,
      notice,
      checkNow,
      install,
      dismiss,
      clearNotice,
      changelogOpen,
      openChangelog,
      closeChangelog,
      aboutOpen,
      openAbout,
      closeAbout,
      notify,
    ],
  );

  return <SuiteContext.Provider value={value}>{children}</SuiteContext.Provider>;
}
