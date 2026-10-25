import { EventType } from "@ag-ui/core"
import { expect, it } from "vitest"
import { z } from "zod"

import "./agUiCompatibility"
import { readingComponents } from "./ReadingCards"

it("retains Tambo's legacy thinking event names alongside the current protocol", () => {
  expect(EventType).toMatchObject({
    THINKING_START: "THINKING_START",
    THINKING_END: "THINKING_END",
    THINKING_TEXT_MESSAGE_START: "THINKING_TEXT_MESSAGE_START",
    THINKING_TEXT_MESSAGE_CONTENT: "THINKING_TEXT_MESSAGE_CONTENT",
    THINKING_TEXT_MESSAGE_END: "THINKING_TEXT_MESSAGE_END",
    REASONING_MESSAGE_CONTENT: "REASONING_MESSAGE_CONTENT",
    TEXT_MESSAGE_CONTENT: "TEXT_MESSAGE_CONTENT",
    TOOL_CALL_ARGS: "TOOL_CALL_ARGS",
    RUN_FINISHED: "RUN_FINISHED",
  })
})

it("exports bounded card schemas through Zod 4's native JSON Schema conversion", () => {
  const highlights = readingComponents[0].propsSchema as z.ZodType
  const schema = z.toJSONSchema(highlights)
  expect(schema.properties?.points).toMatchObject({ type: "array", maxItems: 6 })
  expect(highlights.safeParse({ title: "Article", points: ["An idea"] }).success).toBe(true)
  expect(highlights.safeParse({ title: "Article", points: Array.from({ length: 7 }, () => "An idea") }).success).toBe(false)
})
