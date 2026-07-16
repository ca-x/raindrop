export type DatabaseKind = "sqlite" | "postgres" | "mysql"
export type SetupStep = "database" | "admin"

export const databaseUrls: Record<DatabaseKind, string> = {
  sqlite: "sqlite://data/raindrop.db?mode=rwc",
  postgres: "postgres://user:password@localhost/raindrop",
  mysql: "mysql://user:password@localhost/raindrop",
}

export interface SetupValues {
  token: string
  databaseKind: DatabaseKind
  databaseUrl: string
  username: string
  email: string
  password: string
}

export const initialSetupValues: SetupValues = {
  token: "",
  databaseKind: "sqlite",
  databaseUrl: databaseUrls.sqlite,
  username: "",
  email: "",
  password: "",
}

export function validateDatabase(values: SetupValues): Record<string, string> {
  const fields: Record<string, string> = {}
  if (!values.token.trim()) fields.token = "required"
  if (!values.databaseUrl.trim()) fields.databaseUrl = "required"
  return fields
}

export function validateAdmin(values: SetupValues): Record<string, string> {
  const fields: Record<string, string> = {}
  const username = values.username.trim()
  const usernameLength = [...username].length
  if (
    usernameLength < 3 ||
    usernameLength > 64 ||
    [...username].some((character) => /\s/u.test(character))
  ) {
    fields.username = "invalid"
  }
  if (values.password.length < 12) fields.password = "invalid"
  if (values.email && !/^\S+@\S+\.\S+$/.test(values.email)) fields.email = "invalid"
  return fields
}
