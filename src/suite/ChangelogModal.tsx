import { Fragment, useEffect, useMemo, type ReactNode } from "react";
import { entriesUpTo, parseChangelog } from "./changelog";
import { useSuite } from "./UpdaterProvider";
import type { SuiteLang, SuiteT } from "./messages";

/** `**bold**`, `*italic*` and `` `code` `` inside a line of a changelog entry. */
function inline(text: string): ReactNode[] {
  return text
    .split(/(\*\*[^*]+\*\*|\*[^*]+\*|`[^`]+`)/g)
    .filter(Boolean)
    .map((part, i) => {
      if (part.startsWith("**")) return <strong key={i}>{part.slice(2, -2)}</strong>;
      if (part.startsWith("*")) return <em key={i}>{part.slice(1, -1)}</em>;
      if (part.startsWith("`")) return <code key={i}>{part.slice(1, -1)}</code>;
      return <Fragment key={i}>{part}</Fragment>;
    });
}

/** The small Markdown subset the changelogs use: "### Heading" and "- bullet". */
function Body({ text }: { text: string }) {
  const blocks: ReactNode[] = [];
  let items: string[] = [];
  const flush = () => {
    if (items.length)
      blocks.push(
        <ul key={`ul${blocks.length}`}>
          {items.map((it, i) => (
            <li key={i}>{inline(it)}</li>
          ))}
        </ul>,
      );
    items = [];
  };
  for (const raw of text.split("\n")) {
    const line = raw.trim();
    const heading = /^###\s+(.*)$/.exec(line);
    const bullet = /^[-*]\s+(.*)$/.exec(line);
    if (heading) {
      flush();
      blocks.push(<h4 key={`h${blocks.length}`}>{heading[1]}</h4>);
    } else if (bullet) items.push(bullet[1]);
    else if (line) {
      flush();
      blocks.push(<p key={`p${blocks.length}`}>{inline(line)}</p>);
    }
  }
  flush();
  return <>{blocks}</>;
}

const SEEN_KEY = "suite:last-seen-version";

interface Props {
  t: SuiteT;
  lang: SuiteLang;
  /** The text of CHANGELOG.md and CHANGELOG.fr.md, bundled with the app. */
  changelogs: Record<SuiteLang, string>;
  /** Names the app in the storage key that remembers the last version seen. */
  appId: string;
}

/** The "What's new" window: from Settings, and by itself once after the app has been
 * updated (not on a first install). */
export function ChangelogModal({ t, lang, changelogs, appId }: Props) {
  const { version, changelogOpen, openChangelog, closeChangelog } = useSuite();

  useEffect(() => {
    if (!version) return;
    const key = `${appId}:${SEEN_KEY}`;
    try {
      const last = localStorage.getItem(key);
      if (last && last !== version) openChangelog();
      localStorage.setItem(key, version);
    } catch {
      // Storage blocked: the window is only opened on request.
    }
  }, [version, appId, openChangelog]);

  useEffect(() => {
    if (!changelogOpen) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") closeChangelog();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [changelogOpen, closeChangelog]);

  const entries = useMemo(
    () => (version ? entriesUpTo(parseChangelog(changelogs[lang]), version) : []),
    [changelogs, lang, version],
  );

  if (!changelogOpen) return null;

  return (
    <div className="cl-backdrop">
      <div className="cl-scrim" aria-hidden="true" onClick={closeChangelog} />
      <div
        className="cl-card changelog-card"
        role="dialog"
        aria-modal="true"
        aria-label={t("cl.title")}
      >
        <div className="cl-header">
          <div className="cl-title">{t("cl.title")}</div>
          <button
            type="button"
            className="cl-close"
            onClick={closeChangelog}
            aria-label={t("cl.close")}
          >
            ×
          </button>
        </div>
        <div className="cl-body">
          {entries.length === 0 && <p className="cl-empty">{t("cl.empty")}</p>}
          {entries.map((e) => (
            <section key={e.version} className="cl-entry">
              <h3>
                {`v${e.version}`}
                {e.date && <small className="cl-date">{e.date}</small>}
              </h3>
              <Body text={e.body} />
            </section>
          ))}
        </div>
        <div className="cl-actions">
          <button type="button" className="sb-primary" onClick={closeChangelog}>
            {t("cl.close")}
          </button>
        </div>
      </div>
    </div>
  );
}
