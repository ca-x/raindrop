import { AppShell } from "@astryxdesign/core/AppShell"
import { Banner } from "@astryxdesign/core/Banner"
import { Button } from "@astryxdesign/core/Button"
import { EmptyState } from "@astryxdesign/core/EmptyState"
import { MobileNav, MobileNavToggle } from "@astryxdesign/core/MobileNav"
import { Section } from "@astryxdesign/core/Section"
import { SideNav, SideNavItem, SideNavSection } from "@astryxdesign/core/SideNav"
import { Stack } from "@astryxdesign/core/Stack"
import { Text } from "@astryxdesign/core/Text"
import { useLingui } from "@lingui/react"

import { BrandMark } from "../../shared/brand/BrandMark"

interface ReadyMobilePageProps {
  username: string
  isLoading: boolean
  hasError: boolean
  onLogout: () => void
}

export function ReadyMobilePage({
  username,
  isLoading,
  hasError,
  onLogout,
}: ReadyMobilePageProps) {
  const { i18n } = useLingui()
  const navItems = (
    <SideNavSection title={username}>
      <SideNavItem
        label={i18n._("common.logout")}
        onClick={onLogout}
        className="raindrop-pressable"
      />
    </SideNavSection>
  )
  const sideNavigation = <SideNav>{navItems}</SideNav>
  const navigation = (
    <MobileNav
      header={
        <Stack direction="horizontal" gap={2} align="center">
          <BrandMark size="sm" decorative />
          <Text type="label">Raindrop</Text>
        </Stack>
      }
      label={i18n._("common.menu")}
      className="raindrop-mobile-navigation"
    >
      {navItems}
    </MobileNav>
  )

  return (
    <AppShell
      contentPadding={0}
      height="fill"
      variant="surface"
      sideNav={sideNavigation}
      mobileNav={{ breakpoint: "md", hasToggle: false, content: navigation }}
    >
      <Stack
        height="100%"
        className="raindrop-mobile-ready"
        data-testid="mobile-ready-page"
      >
        <Section
          variant="section"
          padding={3}
          dividers={["bottom"]}
          className="raindrop-mobile-header"
        >
          <Stack direction="horizontal" gap={3} align="center" justify="between">
            <MobileNavToggle
              label={i18n._("common.menu")}
              className="raindrop-pressable"
              style={{ minWidth: 44, minHeight: 44 }}
            />
            <Stack direction="horizontal" gap={2} align="center">
              <BrandMark size="sm" />
              <Text type="label" maxLines={1}>{username}</Text>
            </Stack>
          </Stack>
        </Section>
        {hasError ? <Banner status="error" title={i18n._("login.error")} /> : null}
        <Section variant="transparent" padding={4} className="raindrop-mobile-task">
          <EmptyState
            headingLevel={1}
            title={i18n._("ready.title")}
            description={i18n._("ready.description")}
            actions={
              <Button
                label={i18n._("common.logout")}
                variant="secondary"
                isLoading={isLoading}
                clickAction={onLogout}
                className="raindrop-pressable"
                style={{ minHeight: 44 }}
              />
            }
          />
        </Section>
      </Stack>
    </AppShell>
  )
}
