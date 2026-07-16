import { AppShell } from "@astryxdesign/core/AppShell"
import { Button } from "@astryxdesign/core/Button"
import { Card } from "@astryxdesign/core/Card"
import { Center } from "@astryxdesign/core/Center"
import { Heading } from "@astryxdesign/core/Heading"
import { Stack } from "@astryxdesign/core/Stack"
import { Text } from "@astryxdesign/core/Text"
import { useLingui } from "@lingui/react"

export function LoginPage() {
  const { i18n } = useLingui()
  return (
    <AppShell contentPadding={0} height="fill" mobileNav={false} variant="wash">
      <Center minHeight="100%" width="100%" className="raindrop-auth-frame">
        <Card maxWidth={440} width="100%" padding={8} className="raindrop-auth-card">
          <Stack gap={4}>
            <Stack gap={1}>
              <Text type="supporting" color="accent">
                {i18n._("login.eyebrow")}
              </Text>
              <Heading level={1} textWrap="balance" className="raindrop-reading-heading">
                {i18n._("login.title")}
              </Heading>
              <Text type="body" color="secondary" textWrap="pretty" as="p">
                {i18n._("login.description")}
              </Text>
            </Stack>
            <Button
              label={i18n._("login.continue")}
              variant="primary"
              size="lg"
              style={{ minHeight: 44 }}
            />
          </Stack>
        </Card>
      </Center>
    </AppShell>
  )
}
