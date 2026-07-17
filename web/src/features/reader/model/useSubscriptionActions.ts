import { useCallback, useEffect, useRef } from "react"

import type { ReaderApi } from "./controllerApi"
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
  expireSession: () => void
}

export function useSubscriptionActions({
  api,
  csrfToken,
  createRequestId,
  dispatch,
  expireSession,
}: SubscriptionActionOptions) {
  const controllers = useRef(new Set<AbortController>())

  const runAction = useCallback(
    async <T,>(
      request: (signal: AbortSignal) => Promise<T>,
      success: (value: T) => ReaderAction,
    ) => {
      const controller = new AbortController()
      controllers.current.add(controller)
      dispatch({ type: "mutationErrorCleared" })
      try {
        dispatch(success(await request(controller.signal)))
      } catch (error) {
        if (isAbortError(error)) return
        if (isUnauthenticatedError(error)) return expireSession()
        dispatch({ type: "mutationErrorSet", error: readerErrorMessage(error) })
      } finally {
        controllers.current.delete(controller)
      }
    },
    [dispatch, expireSession],
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

  useEffect(
    () => () => {
      for (const controller of controllers.current) controller.abort()
    },
    [],
  )

  return { addSubscription, deleteSubscription, refreshSubscription }
}
