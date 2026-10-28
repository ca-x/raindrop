import { AppShell } from "@astryxdesign/core/AppShell"
import { useLingui } from "@lingui/react"

import { BrandMark } from "../shared/brand/BrandMark"

export function AppLoading() {
  const { i18n } = useLingui()

  return (
    <AppShell contentPadding={0} height="fill" mobileNav={false} variant="section">
      <section className="raindrop-launch" aria-label={i18n._("app.loading")} aria-busy="true">
        <div className="raindrop-launch-sources" aria-hidden="true">
          <BrandMark size="sm" decorative />
          <div className="raindrop-launch-source-lines">
            {[0, 1, 2, 3, 4].map((index) => (
              <div className="raindrop-launch-source-row" key={index}>
                <span /><span />
              </div>
            ))}
          </div>
        </div>
        <div className="raindrop-launch-queue">
          <div className="raindrop-launch-status" role="status">
            <BrandMark size="sm" decorative />
            <div>
              <strong>Raindrop</strong>
              <p>{i18n._("app.loading")}</p>
            </div>
            <span className="raindrop-launch-indicator" aria-hidden="true" />
          </div>
          <div className="raindrop-launch-entries" aria-hidden="true">
            {[0, 1, 2, 3, 4].map((index) => (
              <div className="raindrop-launch-entry" key={index}>
                <span /><span /><span />
              </div>
            ))}
          </div>
        </div>
        <div className="raindrop-launch-article" aria-hidden="true">
          <div className="raindrop-launch-paper">
            <span /><span /><span />
            <div className="raindrop-launch-paragraph"><span /><span /><span /><span /></div>
            <div className="raindrop-launch-paragraph"><span /><span /><span /></div>
          </div>
        </div>
      </section>
    </AppShell>
  )
}
