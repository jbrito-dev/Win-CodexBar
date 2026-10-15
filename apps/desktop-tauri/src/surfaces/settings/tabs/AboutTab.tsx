import { useEffect, useState } from "react";
import { useLocale } from "../../../hooks/useLocale";
import { useUpdateState } from "../../../hooks/useUpdateState";
import { getAppInfo, openExternalUrl } from "../../../lib/tauri";
import { Field, Select, SettingsGroup, Toggle } from "../../../components/FormControls";
import type { AppInfoBridge, UpdateChannel } from "../../../types/bridge";
import type { LocaleKey } from "../../../i18n/keys";
import type { TabProps } from "../settingsTabs";
import codexbarIcon from "../../../assets/codexbar-icon.png";

// AgentBar is a fork of nesszer/Win-CodexBar (MIT), the Windows port of
// steipete/CodexBar (MIT). See FORK.md.
const REPO_URL = "https://github.com/jbrito-dev/Win-CodexBar";
const UPSTREAM_URL = "https://github.com/nesszer/Win-CodexBar";
const SUBMIT_ISSUE_URL = `${REPO_URL}/issues/new?labels=bug&template=bug_report.yml`;

const ABOUT_LINKS: ReadonlyArray<{ labelKey: LocaleKey; url: string }> = [
  {
    labelKey: "AboutLinkGitHub",
    url: REPO_URL,
  },
  {
    labelKey: "AboutLinkUpstream",
    url: UPSTREAM_URL,
  },
  {
    labelKey: "AboutLinkOriginalProject",
    url: "https://github.com/steipete/CodexBar",
  },
  {
    labelKey: "SubmitIssue",
    url: SUBMIT_ISSUE_URL,
  },
];

export default function AboutTab({ settings, set, saving }: TabProps) {
  const { t } = useLocale();
  const [appInfo, setAppInfo] = useState<AppInfoBridge | null>(null);
  const { updateState, checkNow, download, apply, openRelease } =
    useUpdateState();
  const [hasChecked, setHasChecked] = useState(false);
  const [linkError, setLinkError] = useState<string | null>(null);

  useEffect(() => {
    void getAppInfo().then(setAppInfo);
  }, []);

  const handleCheck = () => {
    setHasChecked(true);
    checkNow();
  };

  const openAboutLink = (url: string) => {
    setLinkError(null);
    openExternalUrl(url).catch((error) => {
      setLinkError(String(error));
    });
  };

  if (!appInfo) {
    return (
      <section className="settings-section">
        <p className="settings-section__hint">{t("AboutLoading")}</p>
      </section>
    );
  }

  const isBusy =
    updateState.status === "checking" ||
    updateState.status === "downloading";

  // Copyright is split into two keys so the brand link can render inline
  // between them, avoiding any Fluent placeholder syntax.
  const copyrightBefore = t("AboutCopyrightBefore");
  const copyrightAfter = t("AboutCopyrightAfter");

  return (
    <div className="about-section">
      <div className="about-hero">
        <img className="about-icon" src={codexbarIcon} alt={t("AppName")} />
        <h2 className="about-hero__name">{appInfo.name}</h2>
        <p className="about-hero__line">
          {t("Version")} {appInfo.version}
          {appInfo.buildNumber !== "dev" && ` (${appInfo.buildNumber})`}
        </p>
        {appInfo.buildDate && (
          <p className="about-hero__line">
            {t("AboutBuilt").replace("{}", appInfo.buildDate)}
          </p>
        )}
        <p className="about-hero__line">{appInfo.tagline}</p>
      </div>

      <SettingsGroup title={t("UpdatesTitle")}>
        <Field
          label={t("AutoDownloadUpdates")}
          description={t("AutoDownloadUpdatesHelper")}
        >
          <Toggle
            variant="switch"
            checked={settings.autoDownloadUpdates}
            ariaLabel={t("AutoDownloadUpdates")}
            disabled={saving}
            onChange={(v) => set({ autoDownloadUpdates: v })}
          />
        </Field>
        <Field
          label={t("UpdateChannelChoice")}
          description={t("UpdateChannelChoiceHelper")}
        >
          <Select
            variant="menu"
            value={settings.updateChannel}
            disabled={saving}
            ariaLabel={t("UpdateChannelChoice")}
            options={[
              { value: "stable", label: t("UpdateChannelStableOption") },
              { value: "beta", label: t("UpdateChannelBetaOption") },
            ]}
            onChange={(v) => set({ updateChannel: v as UpdateChannel })}
          />
        </Field>
        <div className="settings-group__row settings-group__row--split">
          <span className="about-update-msg">
            {t("Version")} {appInfo.version}
          </span>
          <button
            type="button"
            className="credential-btn"
            disabled={isBusy}
            onClick={handleCheck}
          >
            {updateState.status === "checking"
              ? t("AboutChecking")
              : t("AboutCheckForUpdates")}
          </button>
        </div>

        {updateState.status === "available" && (
          <div className="settings-group__row settings-group__row--split">
            <span className="about-update-msg">
              {t("UpdateAvailableMessage").replace(
                "{}",
                updateState.version ?? "",
              )}
            </span>
            {updateState.canDownload ? (
              <button
                type="button"
                className="credential-btn credential-btn--primary"
                onClick={download}
              >
                {t("BannerDownloadButton")}
              </button>
            ) : (
              <button type="button" className="credential-btn" onClick={openRelease}>
                {t("BannerViewRelease")}
              </button>
            )}
          </div>
        )}

        {updateState.status === "downloading" && (
          <div className="settings-group__row">
            <span className="about-update-msg">
              {t("UpdateDownloading")}
              {updateState.progress != null &&
                ` ${Math.round(updateState.progress * 100)}%`}
            </span>
          </div>
        )}

        {updateState.status === "ready" && (
          <div className="settings-group__row settings-group__row--split">
            <span className="about-update-msg">{t("UpdateReady")}</span>
            {updateState.canApply ? (
              <button
                type="button"
                className="credential-btn credential-btn--primary"
                onClick={apply}
              >
                {t("BannerInstallRestart")}
              </button>
            ) : (
              <button type="button" className="credential-btn" onClick={openRelease}>
                {t("BannerViewRelease")}
              </button>
            )}
          </div>
        )}

        {updateState.status === "error" && (
          <div className="settings-group__row">
            <span className="about-update-msg">
              {t("ErrorPrefix")} {updateState.error}
            </span>
          </div>
        )}

        {updateState.status === "idle" && hasChecked && (
          <div className="settings-group__row">
            <span className="about-update-msg">{t("AboutUpToDate")}</span>
          </div>
        )}
      </SettingsGroup>

      <SettingsGroup
        title={t("SectionLinks")}
        footer={
          linkError ? (
            <p className="settings-group__error" role="alert">
              {t("ErrorPrefix")} {linkError}
            </p>
          ) : undefined
        }
      >
        {ABOUT_LINKS.map((link) => (
          <button
            key={link.url}
            type="button"
            className="settings-link-row"
            onClick={() => openAboutLink(link.url)}
          >
            <span>{t(link.labelKey)}</span>
            <span className="settings-link-row__arrow" aria-hidden="true">
              ↗
            </span>
          </button>
        ))}
      </SettingsGroup>

      <p className="about-copyright">{t("AboutForkNotice")}</p>
      <p className="about-copyright">
        {copyrightBefore}{" "}
        <button
          type="button"
          className="about-link about-link--inline"
          onClick={() => openAboutLink("https://github.com/steipete/CodexBar")}
        >
          CodexBar
        </button>
        {" "}{copyrightAfter}
      </p>
    </div>
  );
}
