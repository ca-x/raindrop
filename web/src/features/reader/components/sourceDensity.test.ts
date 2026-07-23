import { describe, expect, it } from "vitest"

import { sourceTreeDensityMetrics } from "./sourceDensity"

describe("sourceTreeDensityMetrics", () => {
  it.each([
    ["compact", 28, 14],
    ["balanced", 36, 16],
    ["spacious", 44, 18],
  ] as const)(
    "keeps %s source rows and icons proportional",
    (density, rowBlockSize, iconSize) => {
      expect(sourceTreeDensityMetrics[density]).toEqual({ rowBlockSize, iconSize })
    },
  )
})
