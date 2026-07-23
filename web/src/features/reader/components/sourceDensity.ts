import type { TreeListDensity } from "@astryxdesign/core/TreeList"
import type { CSSProperties } from "react"

interface SourceTreeDensityMetrics {
  rowBlockSize: number
  iconSize: number
}

export const sourceTreeDensityMetrics: Record<
  TreeListDensity,
  SourceTreeDensityMetrics
> = {
  compact: { rowBlockSize: 28, iconSize: 14 },
  balanced: { rowBlockSize: 36, iconSize: 16 },
  spacious: { rowBlockSize: 44, iconSize: 18 },
}

type SourceTreeDensityStyle = CSSProperties & {
  "--reader-source-row-min-block-size": string
  "--reader-source-icon-size": string
}

export function sourceTreeDensityStyle(
  density: TreeListDensity,
): SourceTreeDensityStyle {
  const metrics = sourceTreeDensityMetrics[density]
  return {
    "--reader-source-row-min-block-size": `${metrics.rowBlockSize}px`,
    "--reader-source-icon-size": `${metrics.iconSize}px`,
  }
}
