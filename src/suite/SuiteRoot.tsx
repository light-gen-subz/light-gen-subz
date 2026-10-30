import type { ReactNode } from "react";
import { AboutModal } from "./AboutModal";
import { ChangelogModal } from "./ChangelogModal";
import { UpdateBanner } from "./UpdateBanner";
import { UpdaterProvider } from "./UpdaterProvider";
import type { SuiteLang, SuiteT } from "./messages";
import "./suite.css";

interface Props {
  lang: SuiteLang;
  t: SuiteT;
  appId: string;
  /** The product name, as written everywhere ("LightScrapVidz"). */
  appName: string;
  changelogs: Record<SuiteLang, string>;
  fetchNotes: (lang: SuiteLang, version: string) => Promise<string>;
  checkOnStart?: boolean;
  beforeRestart?: () => Promise<boolean>;
  children: ReactNode;
}

/** Everything the apps of the suite share: the update check, its banner and the "What's
 * new" window. Mount it once, around the app. */
export function SuiteRoot({
  lang,
  t,
  appId,
  appName,
  changelogs,
  fetchNotes,
  checkOnStart,
  beforeRestart,
  children,
}: Props) {
  return (
    <UpdaterProvider
      lang={lang}
      fetchNotes={fetchNotes}
      checkOnStart={checkOnStart}
      beforeRestart={beforeRestart}
    >
      {children}
      <UpdateBanner t={t} />
      <AboutModal t={t} appName={appName} />
      <ChangelogModal t={t} lang={lang} changelogs={changelogs} appId={appId} />
    </UpdaterProvider>
  );
}
