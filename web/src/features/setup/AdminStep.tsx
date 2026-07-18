import { Button } from "@astryxdesign/core/Button"
import { FormLayout } from "@astryxdesign/core/FormLayout"
import { Heading } from "@astryxdesign/core/Heading"
import { Stack } from "@astryxdesign/core/Stack"
import { Text } from "@astryxdesign/core/Text"
import { TextInput } from "@astryxdesign/core/TextInput"
import { useLingui } from "@lingui/react"
import type { FormEvent } from "react"

import type { SetupValues } from "./model"

interface AdminStepProps {
  values: SetupValues
  fields: Record<string, string>
  isLoading: boolean
  onChange: (next: SetupValues) => void
  onBack?: () => void
  showToken?: boolean
  onSubmit: () => void
}

export function AdminStep({
  values,
  fields,
  isLoading,
  onChange,
  onBack,
  showToken = false,
  onSubmit,
}: AdminStepProps) {
  const { i18n } = useLingui()
  const submit = (event: FormEvent) => {
    event.preventDefault()
    onSubmit()
  }
  return (
    <form onSubmit={submit} noValidate>
      <Stack gap={4}>
        <Stack gap={1}>
          <Heading level={2} className="raindrop-reading-heading">
            {i18n._("setup.adminTitle")}
          </Heading>
          <Text as="p" color="secondary" textWrap="pretty">
            {i18n._("setup.adminDescription")}
          </Text>
        </Stack>
        <FormLayout>
          {showToken ? (
            <TextInput
              label={i18n._("setup.token")}
              description={i18n._("setup.tokenDescription")}
              type="password"
              value={values.token}
              onChange={(token) => onChange({ ...values, token })}
              htmlName="setupToken"
              isRequired
              isDisabled={isLoading}
              width="100%"
              style={{ minHeight: 44 }}
              status={fieldStatus(fields.token, i18n._("setup.required"))}
            />
          ) : null}
          <TextInput
            label={i18n._("setup.username")}
            value={values.username}
            onChange={(username) => onChange({ ...values, username })}
            htmlName="username"
            isRequired
            isDisabled={isLoading}
            width="100%"
            style={{ minHeight: 44 }}
            status={fieldStatus(fields.username, i18n._("setup.usernameInvalid"))}
          />
          <TextInput
            label={i18n._("setup.email")}
            type="email"
            value={values.email}
            onChange={(email) => onChange({ ...values, email })}
            htmlName="email"
            isOptional
            isDisabled={isLoading}
            width="100%"
            style={{ minHeight: 44 }}
            status={fieldStatus(fields.email, i18n._("setup.emailInvalid"))}
          />
          <TextInput
            label={i18n._("setup.password")}
            description={i18n._("setup.passwordDescription")}
            type="password"
            value={values.password}
            onChange={(password) => onChange({ ...values, password })}
            htmlName="password"
            isRequired
            isDisabled={isLoading}
            width="100%"
            style={{ minHeight: 44 }}
            status={fieldStatus(fields.password, i18n._("setup.passwordInvalid"))}
          />
        </FormLayout>
        <Stack direction="horizontal" gap={2} justify="end" wrap="wrap">
          {onBack ? (
            <Button
              type="button"
              label={i18n._("setup.back")}
              variant="secondary"
              onClick={onBack}
              isDisabled={isLoading}
              style={{ minHeight: 44 }}
            />
          ) : null}
          <Button
            type="submit"
            label={i18n._("setup.complete")}
            variant="primary"
            size="lg"
            isLoading={isLoading}
            style={{ minHeight: 44 }}
          />
        </Stack>
      </Stack>
    </form>
  )
}

function fieldStatus(value: string | undefined, message: string) {
  return value ? ({ type: "error", message } as const) : undefined
}
