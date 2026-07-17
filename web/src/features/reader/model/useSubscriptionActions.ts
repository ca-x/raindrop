import { useCallback } from "react"

import type { ReaderApi } from "./controllerApi"
import type { ReaderSession } from "./controllerSession"
import {
  isAbortError,
  isUnauthenticatedError,
  readerErrorMessage,
} from "./controllerErrors"
import type { ReaderAction } from "./reducer"

interface SubscriptionActionOptions {
  api: ReaderApi
  csrfToken: string
  createRequestId: () => string
  dispatch: (action: ReaderAction) => void
  session: ReaderSession
}

export function useSubscriptionActions({
  api,
  csrfToken,
  createRequestId,
  dispatch,
  session,
}: SubscriptionActionOptions) {
  const runAction = useCallback(
    async <T,>(
      request: (signal: AbortSignal) => Promise<T>,
      success: (value: T) => ReaderAction,
    ) => {
      const task = session.begin()
      if (!task) return
      dispatch({ type: "mutationErrorCleared" })
      try {
        const value = await request(task.controller.signal)
        if (session.isCurrent(task)) dispatch(success(value))
      } catch (error) {
        if (isAbortError(error)) return
        if (!session.isCurrent(task)) return
        if (isUnauthenticatedError(error)) return session.expire(task)
        dispatch({ type: "mutationErrorSet", error: readerErrorMessage(error) })
      } finally {
        session.finish(task)
      }
    },
    [dispatch, session],
  )

  const addSubscription = useCallback(
    (url: string) =>
      runAction(
        (signal) => api.createSubscription({ url }, csrfToken, signal),
        (response) => ({
          type: "subscriptionUpserted",
          subscription: response.subscription,
        }),
      ),
    [api, csrfToken, runAction],
  )

  const deleteSubscription = useCallback(
    (subscriptionId: string) =>
      runAction(
        (signal) => api.deleteSubscription(subscriptionId, csrfToken, signal),
        () => ({ type: "subscriptionDeleted", subscriptionId }),
      ),
    [api, csrfToken, runAction],
  )

  const refreshSubscription = useCallback(
    (subscriptionId: string) =>
      runAction(
        (signal) =>
          api.refreshSubscription(
            subscriptionId,
            { requestId: createRequestId() },
            csrfToken,
            signal,
          ),
        (refresh) => ({ type: "subscriptionRefreshUpdated", subscriptionId, refresh }),
      ),
    [api, createRequestId, csrfToken, runAction],
  )
  return { addSubscription, deleteSubscription, refreshSubscription }
}
