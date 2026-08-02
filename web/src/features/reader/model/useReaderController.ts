import { useCallback, useEffect, useReducer, useRef } from "react"

import {
  browserReaderCache,
  readerCacheSnapshot,
  type ReaderCache,
} from "../cache/readerCache"
import { defaultReaderApi, type ReaderApi } from "./controllerApi"
import {
  initialReaderStateForSource,
  readerReducer,
  type ReaderAction,
} from "./reducer"
import { useReaderSession } from "./controllerSession"
import { sourceKey, type ReaderSource, type ReaderState, type SourceKey } from "./types"
import { adjacentUnreadSource, type UnreadSourceDirection } from "./unreadSourceNavigation"
import { useBulkReadActions } from "./useBulkReadActions"
import { useEntryMutations } from "./useEntryMutations"
import { useOrganizationActions } from "./useOrganizationActions"
import { useReaderRequests } from "./useReaderRequests"
import { useSubscriptionActions } from "./useSubscriptionActions"
import type { UpdateCategoryRequest } from "../api/organization.generated"
import type {
  CreateSubscriptionResponse,
  UpdateSubscriptionRequest,
} from "../api/subscription.generated"

export interface ReaderController {
  state: ReaderState
  load: () => Promise<void>
  selectSource: (source: ReaderSource) => Promise<void>
  selectEntry: (entryId: string | null) => Promise<void>
  reloadEntries: () => Promise<void>
  searchFeed: (query: string) => Promise<void>
  mergePendingEntries: () => void
  isMarkingRead: boolean
  markCurrentSourceRead: () => Promise<boolean>
  markFeedRead: (feedId: string) => Promise<boolean>
  nextUnreadSource: () => Promise<void>
  previousUnreadSource: () => Promise<void>
  toggleRead: (entryId: string) => Promise<void>
  toggleStar: (entryId: string) => Promise<void>
  addSubscription: (url: string) => Promise<CreateSubscriptionResponse | null>
  deleteSubscription: (subscriptionId: string) => Promise<boolean>
  refreshSubscription: (subscriptionId: string) => Promise<void>
  createCategory: (title: string) => Promise<boolean>
  updateCategory: (
    categoryId: string,
    request: UpdateCategoryRequest,
  ) => Promise<boolean>
  deleteCategory: (categoryId: string) => Promise<boolean>
  updateSubscription: (
    subscriptionId: string,
    request: UpdateSubscriptionRequest,
  ) => Promise<boolean>
  recordScrollAnchor: (route: string, offset: number) => void
  clearMutationError: () => void
  clearCache: () => Promise<void>
}

export interface UseReaderControllerOptions {
  csrfToken: string
  onUnauthenticated: () => void | Promise<void>
  userId?: string
  initialSource?: ReaderSource
  cache?: ReaderCache
  api?: ReaderApi
  createRequestId?: () => string
}

const cacheSaveDelayMs = 100
const defaultReaderSource: ReaderSource = { kind: "smart", state: "UNREAD" }

interface PendingCacheWrite {
  snapshot: NonNullable<ReturnType<typeof readerCacheSnapshot>>
  markValidated: boolean
}

export function useReaderController({
  csrfToken,
  onUnauthenticated,
  userId,
  initialSource = defaultReaderSource,
  cache = browserReaderCache,
  api = defaultReaderApi,
  createRequestId = defaultRequestId,
}: UseReaderControllerOptions): ReaderController {
  const [state, reactDispatch] = useReducer(
    readerReducer,
    initialReaderStateForSource(initialSource),
  )
  const stateRef = useRef(state)
  stateRef.current = state
  const cacheLoadRef = useRef<Promise<void> | null>(null)
  const loadInvocationRef = useRef(0)
  const cacheLoadSettledRef = useRef(userId === undefined)
  const cacheHydratedRef = useRef(false)
  const cacheWriteEnabledRef = useRef(false)
  const cacheDisabledRef = useRef(false)
  const cacheSaveTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const cacheSaveInFlightRef = useRef(false)
  const pendingCacheWriteRef = useRef<PendingCacheWrite | null>(null)
  const subscriptionsValidatedRef = useRef(false)
  const sourceValidatedKeyRef = useRef<SourceKey | null>(null)
  const cacheClearPromiseRef = useRef<Promise<void> | null>(null)

  const dispatch = useCallback((action: ReaderAction) => {
    stateRef.current = readerReducer(stateRef.current, action)
    reactDispatch(action)
  }, [])

  const clearCache = useCallback((): Promise<void> => {
    if (cacheClearPromiseRef.current) return cacheClearPromiseRef.current
    cacheDisabledRef.current = true
    cacheWriteEnabledRef.current = false
    subscriptionsValidatedRef.current = false
    sourceValidatedKeyRef.current = null
    pendingCacheWriteRef.current = null
    if (cacheSaveTimerRef.current !== null) {
      clearTimeout(cacheSaveTimerRef.current)
      cacheSaveTimerRef.current = null
    }
    const pending = (async () => {
      try {
        await cache.clear()
      } catch {
        // An optional cache delete must never block logout.
      }
    })()
    cacheClearPromiseRef.current = pending
    return pending
  }, [cache])

  const handleUnauthenticated = useCallback(async () => {
    await clearCache()
    await onUnauthenticated()
  }, [clearCache, onUnauthenticated])

  const session = useReaderSession(dispatch, handleUnauthenticated)

  const enqueueCacheWrite = useCallback((write: PendingCacheWrite) => {
    if (cacheDisabledRef.current || !session.active()) return
    if (cacheSaveInFlightRef.current) {
      pendingCacheWriteRef.current = {
        snapshot: write.snapshot,
        markValidated:
          write.markValidated || pendingCacheWriteRef.current?.markValidated === true,
      }
      return
    }
    cacheSaveInFlightRef.current = true
    void (async () => {
      let next: PendingCacheWrite | null = write
      while (next && !cacheDisabledRef.current && session.active()) {
        try {
          await cache.save(
            userId!,
            next.snapshot,
            next.markValidated ? { markValidated: true } : undefined,
          )
        } catch {
          // Injected/custom caches retain the same best-effort boundary.
        }
        next = pendingCacheWriteRef.current
        pendingCacheWriteRef.current = null
      }
      cacheSaveInFlightRef.current = false
    })()
  }, [cache, session, userId])

  const persistCurrentCacheState = useCallback((markValidated = false) => {
    if (
      !userId ||
      !cacheLoadSettledRef.current ||
      !cacheWriteEnabledRef.current ||
      cacheDisabledRef.current ||
      !session.active()
    ) return false
    const snapshot = readerCacheSnapshot(stateRef.current)
    if (!snapshot) return false
    enqueueCacheWrite({ snapshot, markValidated })
    return true
  }, [enqueueCacheWrite, session, userId])

  const persistWhenFullyValidated = useCallback(() => {
    if (!subscriptionsValidatedRef.current) return
    const selectedKey = sourceKey(stateRef.current.selectedSource)
    if (sourceValidatedKeyRef.current !== selectedKey) return
    const wasWriteEnabled = cacheWriteEnabledRef.current
    cacheWriteEnabledRef.current = true
    if (!persistCurrentCacheState(true)) {
      cacheWriteEnabledRef.current = wasWriteEnabled
      sourceValidatedKeyRef.current = null
      return
    }
    subscriptionsValidatedRef.current = false
    sourceValidatedKeyRef.current = null
  }, [persistCurrentCacheState])

  const onSubscriptionsValidated = useCallback(() => {
    subscriptionsValidatedRef.current = true
    persistWhenFullyValidated()
  }, [persistWhenFullyValidated])

  const onSourceValidated = useCallback((source: ReaderSource) => {
    sourceValidatedKeyRef.current = sourceKey(source)
    persistWhenFullyValidated()
  }, [persistWhenFullyValidated])

  const hydrateCache = useCallback(() => {
    if (!userId || cacheDisabledRef.current) {
      cacheLoadSettledRef.current = true
      return Promise.resolve()
    }
    if (cacheLoadRef.current) return cacheLoadRef.current
    cacheLoadRef.current = (async () => {
      try {
        const cached = await cache.load(userId)
        if (
          cached &&
          !cacheHydratedRef.current &&
          !cacheDisabledRef.current &&
          session.active()
        ) {
          cacheHydratedRef.current = true
          cacheWriteEnabledRef.current = true
          dispatch({ type: "readerCacheHydrated", cached })
        }
      } catch {
        // Injected/custom caches retain the same best-effort boundary.
      } finally {
        cacheLoadSettledRef.current = true
      }
    })()
    return cacheLoadRef.current
  }, [cache, dispatch, session, userId])

  const requests = useReaderRequests({
    api,
    dispatch,
    stateRef,
    session,
    userId,
    onSubscriptionsValidated,
    onSourceValidated,
  })
  const scheduleCacheSave = useCallback(() => {
    if (
      !userId ||
      !cacheLoadSettledRef.current ||
      !cacheWriteEnabledRef.current ||
      cacheDisabledRef.current ||
      !session.active()
    ) {
      return
    }
    if (cacheSaveTimerRef.current !== null) clearTimeout(cacheSaveTimerRef.current)
    cacheSaveTimerRef.current = setTimeout(() => {
      cacheSaveTimerRef.current = null
      if (
        cacheDisabledRef.current ||
        !cacheWriteEnabledRef.current ||
        !session.active()
      ) {
        return
      }
      persistCurrentCacheState()
    }, cacheSaveDelayMs)
  }, [persistCurrentCacheState, session, userId])
  const load = useCallback(async () => {
    const invocation = ++loadInvocationRef.current
    if (userId) await hydrateCache()
    if (invocation !== loadInvocationRef.current || !session.active()) return
    await requests.load()
  }, [hydrateCache, requests.load, session, userId])
  const { selectSource } = requests
  const revalidateOrganization = useCallback(() => {
    void Promise.all([
      requests.reloadSubscriptions(),
      requests.replaceEntries(),
    ])
  }, [requests.reloadSubscriptions, requests.replaceEntries])
  const revalidateAfterEntryMutation = useCallback(() => {
    void Promise.all([
      requests.reloadEntries(),
      requests.reloadSelectedEntry(),
    ])
  }, [requests.reloadEntries, requests.reloadSelectedEntry])
  const entryMutations = useEntryMutations({
    api,
    csrfToken,
    dispatch,
    stateRef,
    session,
    onMutationSettled: revalidateAfterEntryMutation,
  })
  const subscriptionActions = useSubscriptionActions({
    api,
    csrfToken,
    createRequestId,
    dispatch,
    session,
    onOrganizationChanged: revalidateOrganization,
  })
  const organizationActions = useOrganizationActions({
    api,
    csrfToken,
    dispatch,
    session,
    onOrganizationChanged: revalidateOrganization,
  })
  const bulkRead = useBulkReadActions({
    api,
    csrfToken,
    dispatch,
    stateRef,
    session,
    userId,
    reloadSubscriptions: requests.reloadSubscriptions,
    replaceEntries: requests.replaceEntries,
  })

  const selectUnreadSource = useCallback(
    async (direction: UnreadSourceDirection) => {
      if (!session.active()) return
      const source = adjacentUnreadSource(stateRef.current, direction)
      if (source) await selectSource(source)
    },
    [selectSource, session, stateRef],
  )

  const recordScrollAnchor = useCallback(
    (route: string, offset: number) => {
      if (!session.active()) return
      dispatch({ type: "scrollAnchorRecorded", route, offset })
    },
    [dispatch, session],
  )
  const clearMutationError = useCallback(() => {
    if (!session.active()) return
    dispatch({ type: "mutationErrorCleared" })
  }, [dispatch, session])

  useEffect(() => {
    scheduleCacheSave()
  }, [scheduleCacheSave, state])

  useEffect(
    () => () => {
      if (cacheSaveTimerRef.current !== null) {
        clearTimeout(cacheSaveTimerRef.current)
        cacheSaveTimerRef.current = null
      }
    },
    [],
  )

  return {
    state,
    ...requests,
    load,
    ...entryMutations,
    ...bulkRead,
    ...subscriptionActions,
    ...organizationActions,
    nextUnreadSource: () => selectUnreadSource(1),
    previousUnreadSource: () => selectUnreadSource(-1),
    recordScrollAnchor,
    clearMutationError,
    clearCache,
  }
}

function defaultRequestId(): string {
  return crypto.randomUUID()
}
