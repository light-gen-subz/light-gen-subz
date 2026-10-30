import type { ReactNode } from "react";
import { SuiteRoot } from "../../suite/SuiteRoot";
import { getT } from "../../i18n";

/** The parts every app of the suite shares (update check, banner, What's new), around `ui`. */
export function withSuite(
  ui: ReactNode,
  {
    fetchNotes = () => Promise.resolve(""),
    changelogs = { en: "", fr: "" },
    lang = "en",
  }: {
    fetchNotes?: (lang: "en" | "fr", version: string) => Promise<string>;
    changelogs?: { en: string; fr: string };
    lang?: "en" | "fr";
  } = {},
) {
  return (
    <SuiteRoot
      lang={lang}
      t={getT(lang)}
      appId="test"
      appName="Test"
      changelogs={changelogs}
      fetchNotes={fetchNotes}
      checkOnStart={false}
    >
      {ui}
    </SuiteRoot>
  );
}
