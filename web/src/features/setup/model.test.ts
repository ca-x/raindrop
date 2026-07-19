import { expect, it } from "vitest"

import { initialSetupValues, validateAdmin } from "./model"

it("accepts a short non-empty password", () => {
  expect(
    validateAdmin({
      ...initialSetupValues,
      username: "Reader",
      password: "a",
    }),
  ).not.toHaveProperty("password")
})

it("rejects an empty password", () => {
  expect(
    validateAdmin({
      ...initialSetupValues,
      username: "Reader",
      password: "",
    }),
  ).toHaveProperty("password", "invalid")
})
