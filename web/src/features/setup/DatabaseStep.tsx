import { Button } from "@astryxdesign/core/Button"
import { Code } from "@astryxdesign/core/Code"
import { FormLayout } from "@astryxdesign/core/FormLayout"
import { Heading } from "@astryxdesign/core/Heading"
import { RadioList, RadioListItem } from "@astryxdesign/core/RadioList"
import { Stack } from "@astryxdesign/core/Stack"
import { Text } from "@astryxdesign/core/Text"
import { TextInput } from "@astryxdesign/core/TextInput"
import { useLingui } from "@lingui/react"
import type { FormEvent } from "react"

import { databaseUrls, type DatabaseKind, type SetupValues } from "./model"

interface DatabaseStepProps {
  values: SetupValues
  fields: Record<string, string>
  isLoading: boolean
  onChange: (next: SetupValues) => void
  onSubmit: () => void
}

export function DatabaseStep({
  values,
  fields,
  isLoading,
  onChange,
  onSubmit,
}: DatabaseStepProps) {
  const { i18n } = useLingui()
  const setKind = (kind: string) => {
    if (isLoading) return
    const databaseKind = kind as DatabaseKind
    onChange({ ...values, databaseKind, databaseUrl: databaseUrls[databaseKind] })
  }
  const submit = (event: FormEvent) => {
    event.preventDefault()
    onSubmit()
  }

  return (
    <form onSubmit={submit} noValidate>
      <Stack gap={4}>
        <Stack gap={1}>
          <Heading level={2} className="raindrop-reading-heading">
            {i18n._("setup.databaseTitle")}
          </Heading>
          <Text as="p" color="secondary" textWrap="pretty">
            {i18n._("setup.databaseDescription")}
          </Text>
        </Stack>
        <FormLayout>
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
          <RadioList
            label={i18n._("setup.databaseType")}
            value={values.databaseKind}
            onChange={setKind}
            htmlName="databaseKind"
            isDisabled={isLoading}
          >
            <RadioListItem
              label="SQLite"
              value="sqlite"
              description="WAL · single node"
              data-testid="database-kind-sqlite"
              style={{ minHeight: 44 }}
              onClick={() => setKind("sqlite")}
            />
            <RadioListItem
              label="PostgreSQL"
              value="postgres"
              description="Shared database"
              data-testid="database-kind-postgres"
              style={{ minHeight: 44 }}
              onClick={() => setKind("postgres")}
            />
            <RadioListItem
              label="MySQL"
              value="mysql"
              description="Shared database"
              data-testid="database-kind-mysql"
              style={{ minHeight: 44 }}
              onClick={() => setKind("mysql")}
            />
          </RadioList>
          <TextInput
            label={i18n._("setup.databaseUrl")}
            description={i18n._("setup.databaseUrlDescription")}
            value={values.databaseUrl}
            onChange={(databaseUrl) => onChange({ ...values, databaseUrl })}
            htmlName="databaseUrl"
            isRequired
            isDisabled={isLoading}
            width="100%"
            style={{ minHeight: 44 }}
            status={fieldStatus(fields.databaseUrl, i18n._("setup.required"))}
          />
        </FormLayout>
        <Text as="p" type="supporting" color="secondary">
          {i18n._("setup.environmentHint")} <Code>RAINDROP_DATABASE_URL</Code>.
        </Text>
        <Button
          type="submit"
          label={i18n._("setup.databaseCheck")}
          variant="primary"
          size="lg"
          isLoading={isLoading}
          style={{ minHeight: 44 }}
        />
      </Stack>
    </form>
  )
}

function fieldStatus(value: string | undefined, message: string) {
  return value ? ({ type: "error", message } as const) : undefined
}
