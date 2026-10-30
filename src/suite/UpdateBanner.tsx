import { useState } from "react";
import { Toast } from "./Toast";
import { useSuite } from "./UpdaterProvider";
import type { SuiteT } from "./messages";

/** What the updater says, shown in the window: the new version as a banner with its notes
 * and the "Update now" / "Later" buttons, and the other messages as a toast. The same
 * behaviour in every app of the suite. */
export function UpdateBanner({ t }: { t: SuiteT }) {
  const { phase, update, progress, notice, install, dismiss, clearNotice } = useSuite();
  const [showNotes, setShowNotes] = useState(false);

  const offered = update && phase !== "idle" && phase !== "checking";
  const busy = phase === "downloading" || phase === "installing";

  let message = "";
  if (update) {
    if (phase === "downloading")
      message =
        progress > 0 ? t("upd.downloadingPct", { percent: progress }) : t("upd.downloading");
    else if (phase === "installing") message = t("upd.installing");
    else message = t("upd.available", { version: update.version });
  }

  return (
    <>
      {offered && (
        <div className="update-banner" role="status">
          <span>{message}</span>
          {!busy && update.notes && (
            <button
              type="button"
              className="sb-ghost update-notes-toggle"
              aria-expanded={showNotes}
              onClick={() => setShowNotes((v) => !v)}
            >
              {showNotes ? t("upd.hideNotes") : t("upd.notes")}
            </button>
          )}
          {!busy && (
            <>
              <button type="button" className="sb-primary" onClick={() => void install()}>
                {t("upd.now")}
              </button>
              <button type="button" className="sb-ghost" onClick={dismiss}>
                {t("upd.later")}
              </button>
            </>
          )}
          {!busy && showNotes && update.notes && <div className="update-notes">{update.notes}</div>}
        </div>
      )}
      {notice && (
        <Toast
          key={notice.message ?? notice.key}
          message={notice.message ?? t(notice.key ?? "")}
          type={notice.type}
          duration={notice.type === "error" ? 6000 : 3500}
          onDone={clearNotice}
        />
      )}
    </>
  );
}
