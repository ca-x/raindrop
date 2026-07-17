import { LayerProvider } from "@astryxdesign/core/Layer"
import { LinkProvider } from "@astryxdesign/core/Link"
import { Theme } from "@astryxdesign/core/theme"
import { neutralTheme } from "@astryxdesign/theme-neutral/built"
import { I18nProvider } from "@lingui/react"
import { forwardRef, type AnchorHTMLAttributes, type ReactNode } from "react"
import { BrowserRouter, Link } from "react-router-dom"

import { i18n } from "../shared/i18n/i18n"

type RouterLinkProps = Omit<AnchorHTMLAttributes<HTMLAnchorElement>, "href"> & {
  href: string
}

const RouterLink = forwardRef<HTMLAnchorElement, RouterLinkProps>(function RouterLink(
  { href, ...props },
  ref,
) {
  return <Link ref={ref} to={href} {...props} />
})

export function Providers({ children }: { children: ReactNode }) {
  return (
    <Theme theme={neutralTheme} mode="system">
      <LayerProvider toast={{ position: "bottomEnd", maxVisible: 3 }}>
        <I18nProvider i18n={i18n}>
          <BrowserRouter>
            <LinkProvider component={RouterLink}>{children}</LinkProvider>
          </BrowserRouter>
        </I18nProvider>
      </LayerProvider>
    </Theme>
  )
}
