import type { getT } from "./i18n";
import type { RecentEntry } from "./recent";
import "./Home.css";

type Props = {
  t: ReturnType<typeof getT>;
  dragging: boolean;
  formats: string[];
  recent: RecentEntry[];
  onBrowse: () => void;
  onOpenRecent: (path: string) => void;
  onRemoveRecent: (path: string) => void;
  onClearRecent: () => void;
};

/** The start screen: the same layout as LightCutVidz (drop frame, then recent files). */
export function Home({
  t,
  dragging,
  formats,
  recent,
  onBrowse,
  onOpenRecent,
  onRemoveRecent,
  onClearRecent,
}: Props) {
  return (
    <div className={`home${dragging ? " home--dragging" : ""}`}>
      <div className="dropzone-frame">
        <svg className="dropzone-icon" viewBox="0 0 24 24" fill="none" aria-hidden="true">
          <rect
            x="2.5"
            y="5"
            width="19"
            height="14"
            rx="3"
            stroke="currentColor"
            strokeWidth="1.6"
          />
          <path
            d="M7 11h4M13 11h4M7 15h7"
            stroke="currentColor"
            strokeWidth="1.6"
            strokeLinecap="round"
          />
        </svg>
        <h2 className="dropzone-title">
          LightGen<span className="dropzone-title-accent">Subz</span>
        </h2>
        <p className="dropzone-subtitle">{dragging ? t("drop.dragging") : t("drop.subtitle")}</p>
        <p className="dropzone-or">{t("drop.or")}</p>
        <button type="button" className="dropzone-btn" onClick={onBrowse}>
          {t("drop.browse")}
        </button>
      </div>

      {recent.length > 0 && (
        <section className="recent" aria-label={t("recent.title")}>
          <div className="recent-head">
            <h3>{t("recent.title")}</h3>
            <button type="button" className="recent-clear" onClick={onClearRecent}>
              {t("recent.clear")}
            </button>
          </div>
          <ul>
            {recent.map((item) => (
              <li key={item.path} className="recent-item">
                <button
                  type="button"
                  className="recent-open"
                  onClick={() => onOpenRecent(item.path)}
                  title={item.path}
                >
                  <span className="recent-kind">{item.name.split(".").pop()?.toUpperCase()}</span>
                  <span className="recent-name">{item.name}</span>
                  <span className="recent-dir">{item.dir}</span>
                </button>
                <button
                  type="button"
                  className="recent-remove"
                  onClick={() => onRemoveRecent(item.path)}
                  aria-label={t("recent.remove")}
                  title={t("recent.remove")}
                >
                  ×
                </button>
              </li>
            ))}
          </ul>
        </section>
      )}

      <p className="dropzone-formats">{formats.join(" · ").toUpperCase()}</p>
    </div>
  );
}
