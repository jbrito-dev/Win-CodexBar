import { Field, Toggle } from "../components/FormControls";
import { useLocale } from "../hooks/useLocale";
import type { SettingsSnapshot, SettingsUpdate } from "../types/bridge";

interface Props {
  settings: SettingsSnapshot;
  saving: boolean;
  set: (patch: SettingsUpdate) => void;
}

/** Settings block for the taskbar-docked meter (separate from the floating bar). */
export default function TaskbarMeterSettingsSection({ settings, saving, set }: Props) {
  const { t } = useLocale();
  return (
    <section className="settings-section">
      <h3 className="settings-section__title">{t("TaskbarMeterSectionTitle")}</h3>
      <div className="settings-section__group">
        <Field
          label={t("TaskbarMeterEnabled")}
          description={t("TaskbarMeterEnabledHelper")}
          leading
        >
          <Toggle
            checked={settings.taskbarMeterEnabled}
            disabled={saving}
            onChange={(v) => set({ taskbarMeterEnabled: v })}
          />
        </Field>
        <Field
          label={t("TaskbarMeterLocked")}
          description={t("TaskbarMeterLockedHelper")}
          leading
        >
          <Toggle
            checked={settings.taskbarMeterLocked}
            disabled={saving || !settings.taskbarMeterEnabled}
            onChange={(v) => set({ taskbarMeterLocked: v })}
          />
        </Field>
      </div>
    </section>
  );
}
