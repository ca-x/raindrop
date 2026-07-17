import type { EntryListState } from "../api/reader.generated"
import type { ReaderSource } from "../model/types"

export interface ReaderRouteMatch {
  source: ReaderSource
  sourcePath: string
  entryId: string | null
}

const smartPaths: Record<string, EntryListState> = {
  "/reader/unread": "UNREAD",
  "/reader/all": "ALL",
  "/reader/starred": "STARRED",
}

export function parseReaderPath(pathname: string): ReaderRouteMatch | null {
  const entryMarker = "/entry/"
  const markerIndex = pathname.indexOf(entryMarker)
  const sourcePath = markerIndex === -1 ? pathname : pathname.slice(0, markerIndex)
  const entryId = markerIndex === -1 ? null : pathname.slice(markerIndex + entryMarker.length)
  if (entryId === "" || entryId?.includes("/")) return null

  const smartState = smartPaths[sourcePath]
  if (smartState) return { source: { kind: "smart", state: smartState }, sourcePath, entryId }

  const feedMatch = /^\/reader\/feed\/([^/]+)$/.exec(sourcePath)
  if (!feedMatch) return null
  return {
    source: { kind: "feed", feedId: decodeURIComponent(feedMatch[1]) },
    sourcePath,
    entryId: entryId ? decodeURIComponent(entryId) : null,
  }
}

export function pathForSource(source: ReaderSource): string {
  if (source.kind === "feed") return `/reader/feed/${encodeURIComponent(source.feedId)}`
  return `/reader/${source.state.toLowerCase()}`
}

export function pathForEntry(sourcePath: string, entryId: string): string {
  return `${sourcePath}/entry/${encodeURIComponent(entryId)}`
}

export function sameReaderSource(left: ReaderSource, right: ReaderSource): boolean {
  return left.kind === right.kind &&
    (left.kind === "feed" ? left.feedId === (right as typeof left).feedId : left.state === (right as typeof left).state)
}
