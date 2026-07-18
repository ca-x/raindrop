import { expect, it } from "vitest"

import { initialSetupValues, validateAdmin } from "./model"

it("measures the password minimum in UTF-8 bytes", () => {
  expect(
    validateAdmin({
      ...initialSetupValues,
      username: "Reader",
      password: "🔒🔒🔒",
    }),
  ).not.toHaveProperty("password")
})
