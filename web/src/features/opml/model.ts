export type OpmlImportMode = "PREVIEW" | "COMMIT"

export interface OpmlImportSummary {
  mode: OpmlImportMode
  outlineCount: number
  validCount: number
  newCount: number
  importedCount: number
  duplicateCount: number
  invalidCount: number
  categoryCount: number
  createdCategoryCount: number
}

export function isOpmlImportSummary(value: unknown): value is OpmlImportSummary {
  if (!isRecord(value)) return false
  return (
    (value.mode === "PREVIEW" || value.mode === "COMMIT") &&
    isCount(value.outlineCount) &&
    isCount(value.validCount) &&
    isCount(value.newCount) &&
    isCount(value.importedCount) &&
    isCount(value.duplicateCount) &&
    isCount(value.invalidCount) &&
    isCount(value.categoryCount) &&
    isCount(value.createdCategoryCount)
  )
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null
}

function isCount(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0
}
