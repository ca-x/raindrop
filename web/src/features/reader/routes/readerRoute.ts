import type { EntryListState } from "../api/reader.generated"
import type { ReaderSource } from "../model/types"

export interface ReaderRouteMatch {
  source: ReaderSource
  sourcePath: string
  entryId: string | null
}

export const DEFAULT_READER_PATH = "/reader/unread"

const smartPaths: Record<string, EntryListState> = {
  [DEFAULT_READER_PATH]: "UNREAD",
  "/reader/all": "ALL",
  "/reader/starred": "STARRED",
}

export function parseReaderPath(pathname: string): ReaderRouteMatch | null {
  const entryMarker = "/entry/"
  const markerIndex = pathname.indexOf(entryMarker)
  const sourcePath = markerIndex === -1 ? pathname : pathname.slice(0, markerIndex)
  const encodedEntryId = markerIndex === -1 ? null : pathname.slice(markerIndex + entryMarker.length)
  if (encodedEntryId === "" || encodedEntryId?.includes("/")) return null
  const entryId = encodedEntryId === null ? null : safeDecode(encodedEntryId)
  if (encodedEntryId !== null && entryId === null) return null

  const smartState = smartPaths[sourcePath]
  if (smartState) return { source: { kind: "smart", state: smartState }, sourcePath, entryId }

  const feedMatch = /^\/reader\/feed\/([^/]+)$/.exec(sourcePath)
  if (feedMatch) {
    const feedId = safeDecode(feedMatch[1])
    if (feedId === null) return null
    return {
      source: { kind: "feed", feedId },
      sourcePath,
      entryId,
    }
  }

  const categoryMatch = /^\/reader\/category\/([^/]+)$/.exec(sourcePath)
  if (categoryMatch) {
    const categoryId = safeDecode(categoryMatch[1])
    if (categoryId === null) return null
    return {
      source: { kind: "category", categoryId },
      sourcePath,
      entryId,
    }
  }
  return null
}

export function pathForSource(source: ReaderSource): string {
  if (source.kind === "feed") return `/reader/feed/${encodeURIComponent(source.feedId)}`
  if (source.kind === "category") {
    return `/reader/category/${encodeURIComponent(source.categoryId)}`
  }
  return `/reader/${source.state.toLowerCase()}`
}

export function pathForEntry(sourcePath: string, entryId: string): string {
  return `${sourcePath}/entry/${encodeURIComponent(entryId)}`
}

export function sameReaderSource(left: ReaderSource, right: ReaderSource): boolean {
  if (left.kind !== right.kind) return false
  switch (left.kind) {
    case "smart":
      return left.state === (right as typeof left).state
    case "feed":
      return left.feedId === (right as typeof left).feedId
    case "category":
      return left.categoryId === (right as typeof left).categoryId
  }
}

function safeDecode(value: string): string | null {
  try {
    return decodeURIComponent(value)
  } catch {
    return null
  }
}
