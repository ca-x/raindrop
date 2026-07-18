import { useCallback, useReducer, useRef } from "react"

import { defaultReaderApi, type ReaderApi } from "./controllerApi"
import { initialReaderState, readerReducer, type ReaderAction } from "./reducer"
import { useReaderSession } from "./controllerSession"
import type { ReaderSource, ReaderState } from "./types"
import { adjacentUnreadSource, type UnreadSourceDirection } from "./unreadSourceNavigation"
import { useBulkReadActions } from "./useBulkReadActions"
import { useEntryMutations } from "./useEntryMutations"
import { useOrganizationActions } from "./useOrganizationActions"
import { useReaderRequests } from "./useReaderRequests"
import { useSubscriptionActions } from "./useSubscriptionActions"
import type { UpdateCategoryRequest } from "../api/organization.generated"
import type { UpdateSubscriptionRequest } from "../api/subscription.generated"

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
  nextUnreadSource: () => Promise<void>
  previousUnreadSource: () => Promise<void>
  toggleRead: (entryId: string) => Promise<void>
  toggleStar: (entryId: string) => Promise<void>
  addSubscription: (url: string) => Promise<void>
  deleteSubscription: (subscriptionId: string) => Promise<void>
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
  stateRef.current = state

  const dispatch = useCallback((action: ReaderAction) => {
    stateRef.current = readerReducer(stateRef.current, action)
    reactDispatch(action)
  }, [])

  const session = useReaderSession(dispatch, onUnauthenticated)

  const requests = useReaderRequests({ api, dispatch, stateRef, session })
  const { selectSource } = requests
  const entryMutations = useEntryMutations({
    api,
    csrfToken,
    dispatch,
    stateRef,
    session,
  })
  const subscriptionActions = useSubscriptionActions({
    api,
    csrfToken,
    createRequestId,
    dispatch,
    session,
  })
  const organizationActions = useOrganizationActions({
    api,
    csrfToken,
    dispatch,
    session,
  })
  const bulkRead = useBulkReadActions({
    api,
    csrfToken,
    dispatch,
    stateRef,
    session,
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

  return {
    state,
    ...requests,
    ...entryMutations,
    ...bulkRead,
    ...subscriptionActions,
    ...organizationActions,
    nextUnreadSource: () => selectUnreadSource(1),
    previousUnreadSource: () => selectUnreadSource(-1),
    recordScrollAnchor,
    clearMutationError,
  }
}

function defaultRequestId(): string {
  return crypto.randomUUID()
}
