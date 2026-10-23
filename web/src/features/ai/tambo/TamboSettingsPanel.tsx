import { Banner } from "@astryxdesign/core/Banner"
import { Button } from "@astryxdesign/core/Button"
import { Spinner } from "@astryxdesign/core/Spinner"
import { Switch } from "@astryxdesign/core/Switch"
import { useLingui } from "@lingui/react"
import { useId } from "react"

import { useTamboSettings } from "./useTamboSettings"

export function TamboSettingsPanel({ userId, csrfToken }: { userId: string; csrfToken: string }) {
  const { i18n } = useLingui()
  const headingId = useId()
  const controller = useTamboSettings(userId)
  const { settings } = controller
  return (
    <section className="tambo-settings" aria-labelledby={headingId}>
      <div className="tambo-settings-heading">
        <h4 id={headingId}>{i18n._("tambo.settingsTitle")}</h4>
        <span className="tambo-optional">{i18n._("plugins.setupOptional")}</span>
      </div>
      <p className="reader-preference-description">{i18n._("tambo.settingsDescription")}</p>
      {controller.error ? <Banner status="warning" title={i18n._(`tambo.error.${controller.error}`)} endContent={
        <Button label={i18n._("common.retry")} variant="secondary" onClick={controller.reload} />
      } /> : null}
      {!settings && !controller.error ? <Spinner label={i18n._("tambo.loading")} /> : null}
      {settings ? <>
        <Switch label={i18n._("tambo.enable")} value={settings.enabled}
          onChange={(enabled) => { void controller.save(enabled, csrfToken) }}
          isDisabled={controller.isSaving || (!settings.configured && !settings.enabled)} labelSpacing="spread" />
        {controller.isSaving ? <p role="status">{i18n._("tambo.saving")}</p> : null}
        <p className="reader-preference-description">{i18n._("tambo.persistence")}</p>
        {settings.configured ? <p className="reader-preference-description">{i18n._("tambo.destination", { origin: settings.origin ?? "" })}</p> : (
          <details className="tambo-deployment">
            <summary>{i18n._("tambo.configure")}</summary>
            <p>{i18n._("tambo.deploymentDescription")}</p>
            <code>RAINDROP_TAMBO_URL</code><br /><code>RAINDROP_TAMBO_API_KEY</code>
          </details>
        )}
      </> : null}
    </section>
  )
}
