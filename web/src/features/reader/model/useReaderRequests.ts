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
  loadAllSubscriptions,
  loadCategories,
  ReaderResponseOwnerMismatchError,
  sameSource,
} from "./readerRequestData"
import type { ReaderSource, ReaderState } from "./types"

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
      const [categories, subscriptions] = await Promise.all([
        loadCategories(api, controller.signal, userId),
        loadAllSubscriptions(api, controller.signal, current, userId),
      ])
      if (!current()) return false
      if (
        categories.ownerUserId !== subscriptions.ownerUserId ||
        (userId !== undefined && categories.ownerUserId !== userId)
      ) {
        await session.expire(request)
        return false
      }
      dispatch({
        type: "subscriptionsReceived",
        generation,
        subscriptions: subscriptions.items,
        categories: categories.items,
      })
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
      if (!await loadSubscriptions()) return false
      const source = stateRef.current.selectedSource
      const searchQuery = stateRef.current.feedSearchQuery
      const sourceLoaded = await loadSource(source, "replace", searchQuery)
      return sourceLoaded && searchQuery === ""
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
    async () => { await loadSource(stateRef.current.selectedSource, "discover") },
    [loadSource, stateRef],
  )

  const replaceEntries = useCallback(
    async () => { await loadSource(stateRef.current.selectedSource, "replace") },
    [loadSource, stateRef],
  )

  const reloadSubscriptions = useCallback(
    async () => { await loadSubscriptions() },
    [loadSubscriptions],
  )

  const reloadSelectedEntry = useCallback(
    async () => {
      const entryId = stateRef.current.selectedEntryId
      if (entryId !== null) await selectEntry(entryId)
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
    reloadSelectedEntry,
    searchFeed,
    mergePendingEntries,
  }
}
