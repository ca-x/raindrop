import { describe, expect, it } from "vitest"

import {
  entryQueueDensityMetrics,
  entryQueueDensityStyle,
} from "./entryDensity"

describe("entryQueueDensityMetrics", () => {
  it.each([
    ["compact", 64, 4],
    ["balanced", 76, 8],
    ["spacious", 88, 12],
  ] as const)("keeps %s entry rows proportional", (density, rowBlockSize, paddingBlock) => {
    expect(entryQueueDensityMetrics[density]).toEqual({ rowBlockSize, paddingBlock })
    expect(entryQueueDensityStyle(density)).toEqual({
      "--reader-entry-min-block-size": `${rowBlockSize}px`,
      "--reader-entry-padding-block": `${paddingBlock}px`,
      "--reader-entry-content-min-block-size": `${rowBlockSize - paddingBlock * 2}px`,
    })
  })
})
