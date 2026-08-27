import { useCallback, useEffect, useRef } from "react"

import type { ReaderApi } from "./controllerApi"
import type { ReaderSession } from "./controllerSession"
import {
  isAbortError,
  isUnauthenticatedError,
  readerErrorMessage,
} from "./controllerErrors"
import type { ReaderAction } from "./reducer"
import {
  entryListOptions,
  loadCategories,
  ReaderResponseOwnerMismatchError,
  sameSource,
} from "./readerRequestData"
import { sourceKey, type ReaderSource, type ReaderState } from "./types"

interface ReaderRequestOptions {
  api: ReaderApi
  dispatch: (action: ReaderAction) => void
  stateRef: { current: ReaderState }
  session: ReaderSession
  userId?: string
  onSubscriptionsValidated?: () => void
  onSourceValidated?: (source: ReaderSource) => void
}

type Pane = "subscriptions" | "queue" | "detail"

// Keep the first page at the API default for a fast first paint, then drain
// the remaining pages in larger batches so accounts with hundreds of feeds do
// not spend one round-trip per small page before the projection is complete.
const SUBSCRIPTION_REVALIDATION_PAGE_LIMIT = 100

export function useReaderRequests({
  api,
  dispatch,
  stateRef,
  session,
  userId,
  onSubscriptionsValidated,
  onSourceValidated,
}: ReaderRequestOptions) {
  const controllers = useRef<Partial<Record<Pane, AbortController>>>({})

  const beginRequest = useCallback(
    (pane: Pane) => {
      controllers.current[pane]?.abort()
      const task = session.begin()
      if (!task) return null
      controllers.current[pane] = task.controller
      const generation = stateRef.current.requestGenerationByPane[pane] + 1
      return { ...task, generation }
    },
    [session, stateRef],
  )

  const loadSubscriptions = useCallback(async (): Promise<boolean> => {
    const request = beginRequest("subscriptions")
    if (!request) return false
    const { controller, generation } = request
    const current = () =>
      session.isCurrent(request) &&
      stateRef.current.requestGenerationByPane.subscriptions === generation
    dispatch({ type: "subscriptionsRequested", generation })
    try {
      const subscriptions = []
      let ownerUserId: string | null = null
      const [categories, firstPage] = await Promise.all([
        loadCategories(api, controller.signal, userId),
        api.listSubscriptions({ signal: controller.signal }),
      ])
      let page = firstPage
      while (true) {
        if (!current()) return false
        if (
          categories.ownerUserId !== page.ownerUserId ||
          (userId !== undefined && page.ownerUserId !== userId) ||
          (ownerUserId !== null && ownerUserId !== page.ownerUserId)
        ) {
          await session.expire(request)
          return false
        }
        ownerUserId = page.ownerUserId
        subscriptions.push(...page.items)
        const isFinal = page.nextCursor === null
        dispatch({
          type: "subscriptionsReceived",
          generation,
          subscriptions: [...subscriptions],
          categories: categories.items,
          isFinal,
        })
        if (page.nextCursor === null) break
        page = await api.listSubscriptions({
          cursor: page.nextCursor,
          limit: SUBSCRIPTION_REVALIDATION_PAGE_LIMIT,
          signal: controller.signal,
        })
      }
      onSubscriptionsValidated?.()
      return true
    } catch (error) {
      if (isAbortError(error)) return false
      if (!current()) return false
      if (
        isUnauthenticatedError(error) ||
        error instanceof ReaderResponseOwnerMismatchError
      ) {
        await session.expire(request)
        return false
      }
      dispatch({
        type: "subscriptionsFailed",
        generation,
        error: readerErrorMessage(error),
      })
      return false
    } finally {
      session.finish(request)
    }
  }, [
    api,
    beginRequest,
    dispatch,
    onSubscriptionsValidated,
    session,
    stateRef,
    userId,
  ])

  const loadSource = useCallback(
    async (
      source: ReaderSource,
      mode: "replace" | "discover",
      searchQuery = stateRef.current.feedSearchQuery,
    ): Promise<boolean> => {
      const request = beginRequest("queue")
      if (!request) return false
      const { controller, generation } = request
      const current = () =>
        session.isCurrent(request) &&
        stateRef.current.requestGenerationByPane.queue === generation &&
        sameSource(stateRef.current.selectedSource, source)
      dispatch({ type: "sourceRequested", source, generation })
      try {
        const page = await api.listEntries({
          ...entryListOptions(source, searchQuery),
          signal: controller.signal,
        })
        if (!current()) return false
        if (userId !== undefined && page.ownerUserId !== userId) {
          await session.expire(request)
          return false
        }
        dispatch({
          type: "sourceReceived",
          source,
          generation,
          entries: page.items,
          snapshotGeneration: page.snapshotGeneration,
          nextCursor: page.nextCursor,
          mode,
        })
        if (searchQuery === "") onSourceValidated?.(source)
        return true
      } catch (error) {
        if (isAbortError(error)) return false
        if (!current()) return false
        if (isUnauthenticatedError(error)) {
          await session.expire(request)
          return false
        }
        dispatch({
          type: "sourceFailed",
          source,
          generation,
          error: readerErrorMessage(error),
        })
        return false
      } finally {
        session.finish(request)
      }
    },
    [api, beginRequest, dispatch, onSourceValidated, session, stateRef, userId],
  )

  const load = useCallback(
    async () => {
      const source = stateRef.current.selectedSource
      const searchQuery = stateRef.current.feedSearchQuery
      const [subscriptionsLoaded, sourceLoaded] = await Promise.all([
        loadSubscriptions(),
        loadSource(source, "replace", searchQuery),
      ])
      return subscriptionsLoaded && sourceLoaded && searchQuery === ""
    },
    [loadSource, loadSubscriptions, stateRef],
  )

  const selectSource = useCallback(
    async (source: ReaderSource) => {
      if (!session.active()) return
      controllers.current.detail?.abort()
      dispatch({ type: "sourceSelected", source })
      await loadSource(source, "replace", "")
    },
    [dispatch, loadSource, session],
  )

  const selectEntry = useCallback(
    async (entryId: string | null) => {
      if (!session.active()) return
      dispatch({ type: "entrySelected", entryId })
      if (entryId === null) {
        controllers.current.detail?.abort()
        return
      }
      const request = beginRequest("detail")
      if (!request) return
      const { controller, generation } = request
      const current = () =>
        session.isCurrent(request) &&
        stateRef.current.requestGenerationByPane.detail === generation &&
        stateRef.current.selectedEntryId === entryId
      dispatch({ type: "detailRequested", entryId, generation })
      try {
        const detail = await api.getEntry(entryId, controller.signal)
        if (current()) dispatch({ type: "detailReceived", entryId, generation, detail })
      } catch (error) {
        if (isAbortError(error)) return
        if (!current()) return
        if (isUnauthenticatedError(error)) return session.expire(request)
        dispatch({
          type: "detailFailed",
          entryId,
          generation,
          error: readerErrorMessage(error),
        })
      } finally {
        session.finish(request)
      }
    },
    [api, beginRequest, dispatch, session, stateRef],
  )

  const reloadEntries = useCallback(
    async () => loadSource(stateRef.current.selectedSource, "discover"),
    [loadSource, stateRef],
  )

  const replaceEntries = useCallback(
    async () => loadSource(stateRef.current.selectedSource, "replace"),
    [loadSource, stateRef],
  )

  const reloadSubscriptions = useCallback(
    async () => loadSubscriptions(),
    [loadSubscriptions],
  )

  const loadMoreEntries = useCallback(async () => {
    const source = stateRef.current.selectedSource
    const key = sourceKey(source)
    const cursor = stateRef.current.nextCursorBySourceKey[key]
    if (
      !session.active() ||
      !cursor ||
      stateRef.current.requestActivity.queue ||
      stateRef.current.requestActivity.page ||
      stateRef.current.paneStatus.queue !== "ready"
    ) {
      return false
    }
    const request = beginRequest("queue")
    if (!request) return false
    const { controller, generation } = request
    const current = () =>
      session.isCurrent(request) &&
      stateRef.current.requestGenerationByPane.queue === generation &&
      sameSource(stateRef.current.selectedSource, source)
    dispatch({ type: "sourcePageRequested", source, generation })
    try {
      const page = await api.listEntries({
        ...entryListOptions(source, stateRef.current.feedSearchQuery),
        cursor,
        signal: controller.signal,
      })
      if (!current()) return false
      if (userId !== undefined && page.ownerUserId !== userId) {
        await session.expire(request)
        return false
      }
      dispatch({
        type: "sourcePageReceived",
        source,
        generation,
        entries: page.items,
        snapshotGeneration: page.snapshotGeneration,
        nextCursor: page.nextCursor,
      })
      return true
    } catch (error) {
      if (isAbortError(error) || !current()) return false
      if (isUnauthenticatedError(error)) {
        await session.expire(request)
        return false
      }
      dispatch({
        type: "sourcePageFailed",
        source,
        generation,
        error: readerErrorMessage(error),
      })
      return false
    } finally {
      session.finish(request)
    }
  }, [api, beginRequest, dispatch, session, stateRef, userId])

  const reloadSelectedEntry = useCallback(
    async () => {
      const entryId = stateRef.current.selectedEntryId
      if (entryId === null) return true
      await selectEntry(entryId)
      return true
    },
    [selectEntry, stateRef],
  )

  const searchFeed = useCallback(
    async (query: string) => {
      const source = stateRef.current.selectedSource
      if (!session.active() || source.kind !== "feed") return
      const normalized = query.trim()
      dispatch({ type: "feedSearchChanged", query: normalized })
      await loadSource(source, "replace", normalized)
    },
    [dispatch, loadSource, session, stateRef],
  )

  const mergePendingEntries = useCallback(() => {
    if (!session.active()) return
    dispatch({ type: "pendingEntriesMerged", source: stateRef.current.selectedSource })
  }, [dispatch, session, stateRef])

  useEffect(
    () => () => {
      for (const controller of Object.values(controllers.current)) controller.abort()
    },
    [],
  )

  return {
    load,
    selectSource,
    selectEntry,
    reloadEntries,
    replaceEntries,
    reloadSubscriptions,
    loadMoreEntries,
    reloadSelectedEntry,
    searchFeed,
    mergePendingEntries,
  }
}
