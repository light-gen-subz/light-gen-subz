import { useEffect } from "react";
import { useSuite } from "./UpdaterProvider";
import type { SuiteT } from "./messages";

/** The "About" window of the Help menu: name, what the app is, version, licence. */
export function AboutModal({ t, appName }: { t: SuiteT; appName: string }) {
  const { aboutOpen, closeAbout, version } = useSuite();

  useEffect(() => {
    if (!aboutOpen) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") closeAbout();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [aboutOpen, closeAbout]);

  if (!aboutOpen) return null;

  return (
    <div className="cl-backdrop">
      <div className="cl-scrim" aria-hidden="true" onClick={closeAbout} />
      <div
        className="cl-card"
        role="dialog"
        aria-modal="true"
        aria-label={t("menu.about", { name: appName })}
      >
        <div className="cl-title">{appName}</div>
        <div className="about-tagline">{t("about.tagline")}</div>
        <div className="about-version">{version && t("about.version", { version })}</div>
        <div className="about-licenses">{t("about.licence")}</div>
        <button type="button" className="sb-primary about-close" onClick={closeAbout}>
          {t("about.close")}
        </button>
      </div>
    </div>
  );
}
