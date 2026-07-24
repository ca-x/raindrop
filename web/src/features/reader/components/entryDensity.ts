import type { ListDensity } from "@astryxdesign/core/List"
import type { CSSProperties } from "react"

interface EntryQueueDensityMetrics {
  rowBlockSize: number
  paddingBlock: number
}

export const entryQueueDensityMetrics: Record<
  ListDensity,
  EntryQueueDensityMetrics
> = {
  compact: { rowBlockSize: 64, paddingBlock: 4 },
  balanced: { rowBlockSize: 76, paddingBlock: 8 },
  spacious: { rowBlockSize: 88, paddingBlock: 12 },
}

type EntryQueueDensityStyle = CSSProperties & {
  "--reader-entry-min-block-size": string
  "--reader-entry-padding-block": string
  "--reader-entry-content-min-block-size": string
}

export function entryQueueDensityStyle(
  density: ListDensity,
): EntryQueueDensityStyle {
  const metrics = entryQueueDensityMetrics[density]
  return {
    "--reader-entry-min-block-size": `${metrics.rowBlockSize}px`,
    "--reader-entry-padding-block": `${metrics.paddingBlock}px`,
    "--reader-entry-content-min-block-size": `${metrics.rowBlockSize - metrics.paddingBlock * 2}px`,
  }
}
