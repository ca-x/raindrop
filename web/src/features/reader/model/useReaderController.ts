import { useCallback, useReducer, useRef } from "react"

import { defaultReaderApi, type ReaderApi } from "./controllerApi"
import { initialReaderState, readerReducer, type ReaderAction } from "./reducer"
import type { ReaderSource, ReaderState } from "./types"
import { useEntryMutations } from "./useEntryMutations"
import { useReaderRequests } from "./useReaderRequests"
import { useSubscriptionActions } from "./useSubscriptionActions"

export interface ReaderController {
  state: ReaderState
  load: () => Promise<void>
  selectSource: (source: ReaderSource) => Promise<void>
  selectEntry: (entryId: string | null) => Promise<void>
  reloadEntries: () => Promise<void>
  mergePendingEntries: () => void
  toggleRead: (entryId: string) => Promise<void>
  toggleStar: (entryId: string) => Promise<void>
  addSubscription: (url: string) => Promise<void>
  deleteSubscription: (subscriptionId: string) => Promise<void>
  refreshSubscription: (subscriptionId: string) => Promise<void>
  recordScrollAnchor: (route: string, offset: number) => void
  clearMutationError: () => void
}

export interface UseReaderControllerOptions {
  csrfToken: string
  onUnauthenticated: () => void
  api?: ReaderApi
  createRequestId?: () => string
}

export function useReaderController({
  csrfToken,
  onUnauthenticated,
  api = defaultReaderApi,
  createRequestId = defaultRequestId,
}: UseReaderControllerOptions): ReaderController {
  const [state, reactDispatch] = useReducer(readerReducer, initialReaderState)
  const stateRef = useRef(state)
  const unauthenticatedNotified = useRef(false)
  stateRef.current = state

  const dispatch = useCallback((action: ReaderAction) => {
    stateRef.current = readerReducer(stateRef.current, action)
    reactDispatch(action)
  }, [])

  const expireSession = useCallback(() => {
    dispatch({ type: "sessionExpired" })
    if (!unauthenticatedNotified.current) {
      unauthenticatedNotified.current = true
      onUnauthenticated()
    }
  }, [dispatch, onUnauthenticated])

  const requests = useReaderRequests({ api, dispatch, stateRef, expireSession })
  const entryMutations = useEntryMutations({
    api,
    csrfToken,
    dispatch,
    stateRef,
    expireSession,
  })
  const subscriptionActions = useSubscriptionActions({
    api,
    csrfToken,
    createRequestId,
    dispatch,
    expireSession,
  })

  const recordScrollAnchor = useCallback(
    (route: string, offset: number) => {
      dispatch({ type: "scrollAnchorRecorded", route, offset })
    },
    [dispatch],
  )
  const clearMutationError = useCallback(() => {
    dispatch({ type: "mutationErrorCleared" })
  }, [dispatch])

  return {
    state,
    ...requests,
    ...entryMutations,
    ...subscriptionActions,
    recordScrollAnchor,
    clearMutationError,
  }
}

function defaultRequestId(): string {
  return crypto.randomUUID()
}
