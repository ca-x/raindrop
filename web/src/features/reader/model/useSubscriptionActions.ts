import { useCallback, useEffect, useRef } from "react"

import type { ReaderApi } from "./controllerApi"
import type { ReaderSession } from "./controllerSession"
import {
  isAbortError,
  isUnauthenticatedError,
  readerErrorMessage,
} from "./controllerErrors"
import type { ReaderAction } from "./reducer"
import type { CreateSubscriptionResponse } from "../api/subscription.generated"

interface SubscriptionActionOptions {
  api: ReaderApi
  csrfToken: string
  createRequestId: () => string
  dispatch: (action: ReaderAction) => void
  session: ReaderSession
}

const refreshPollIntervalMs = 1_000
const refreshPollAttempts = 60

export function useSubscriptionActions({
  api,
  csrfToken,
  createRequestId,
  dispatch,
  session,
}: SubscriptionActionOptions) {
  const pollControllers = useRef(new Map<string, AbortController>())
  const cancelSubscriptionPoll = useCallback((subscriptionId: string) => {
    pollControllers.current.get(subscriptionId)?.abort()
    pollControllers.current.delete(subscriptionId)
  }, [])
  const pollSubscription = useCallback(
    (subscriptionId: string) => {
      cancelSubscriptionPoll(subscriptionId)
      const task = session.begin()
      if (!task) return
      pollControllers.current.set(subscriptionId, task.controller)
      void (async () => {
        try {
          for (let attempt = 0; attempt < refreshPollAttempts; attempt += 1) {
            if (attempt > 0) {
              await waitForRefreshPoll(task.controller.signal)
            }
            try {
              const subscription = await api.getSubscription(
                subscriptionId,
                task.controller.signal,
              )
              if (task.controller.signal.aborted || !session.isCurrent(task)) return
              dispatch({ type: "subscriptionUpserted", subscription })
              if (subscription.refresh?.state !== "PENDING") return
            } catch (error) {
              if (isAbortError(error)) return
              if (!session.isCurrent(task)) return
              if (isUnauthenticatedError(error)) return session.expire(task)
              if (attempt === refreshPollAttempts - 1) {
                dispatch({ type: "mutationErrorSet", error: readerErrorMessage(error) })
              }
            }
          }
        } catch (error) {
          if (isAbortError(error)) return
          if (!session.isCurrent(task)) return
          if (isUnauthenticatedError(error)) return session.expire(task)
          dispatch({ type: "mutationErrorSet", error: readerErrorMessage(error) })
        } finally {
          session.finish(task)
          if (pollControllers.current.get(subscriptionId) === task.controller) {
            pollControllers.current.delete(subscriptionId)
          }
        }
      })()
    },
    [api, cancelSubscriptionPoll, dispatch, session],
  )

  const runAction = useCallback(
    async <T,>(
      request: (signal: AbortSignal) => Promise<T>,
      success: (value: T) => ReaderAction,
      followUp?: (value: T) => void,
    ): Promise<boolean> => {
      const task = session.begin()
      if (!task) return false
      dispatch({ type: "mutationErrorCleared" })
      try {
        const value = await request(task.controller.signal)
        if (session.isCurrent(task)) {
          dispatch(success(value))
          followUp?.(value)
          return true
        }
        return false
      } catch (error) {
        if (isAbortError(error) || !session.isCurrent(task)) return false
        if (isUnauthenticatedError(error)) {
          session.expire(task)
          return false
        }
        dispatch({ type: "mutationErrorSet", error: readerErrorMessage(error) })
        return false
      } finally {
        session.finish(task)
      }
    },
    [dispatch, session],
  )

  const addSubscription = useCallback(
    async (url: string) => {
      let added: CreateSubscriptionResponse | null = null
      await runAction(
        (signal) => api.createSubscription({ url }, csrfToken, signal),
        (response) => ({
          type: "subscriptionUpserted",
          subscription: response.subscription,
        }),
        (response) => {
          added = response
          if (response.subscription.refresh?.state === "PENDING") {
            pollSubscription(response.subscription.subscriptionId)
          }
        },
      )
      return added
    },
    [api, csrfToken, pollSubscription, runAction],
  )

  const deleteSubscription = useCallback(
    (subscriptionId: string) =>
      runAction(
        (signal) => api.deleteSubscription(subscriptionId, csrfToken, signal),
        () => ({ type: "subscriptionDeleted", subscriptionId }),
        () => cancelSubscriptionPoll(subscriptionId),
      ),
    [api, cancelSubscriptionPoll, csrfToken, runAction],
  )

  const refreshSubscription = useCallback(
    async (subscriptionId: string) => {
      await runAction(
        (signal) =>
          api.refreshSubscription(
            subscriptionId,
            { requestId: createRequestId() },
            csrfToken,
            signal,
        ),
        (refresh) => ({ type: "subscriptionRefreshUpdated", subscriptionId, refresh }),
        (refresh) => {
          if (refresh.state === "PENDING") pollSubscription(subscriptionId)
        },
      )
    },
    [api, createRequestId, csrfToken, pollSubscription, runAction],
  )

  useEffect(
    () => () => {
      for (const controller of pollControllers.current.values()) controller.abort()
      pollControllers.current.clear()
    },
    [],
  )

  return { addSubscription, deleteSubscription, refreshSubscription }
}

function waitForRefreshPoll(signal: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    if (signal.aborted) {
      reject(new DOMException("The operation was aborted", "AbortError"))
      return
    }
    const onAbort = () => {
      window.clearTimeout(timer)
      reject(new DOMException("The operation was aborted", "AbortError"))
    }
    const timer = window.setTimeout(() => {
      signal.removeEventListener("abort", onAbort)
      resolve()
    }, refreshPollIntervalMs)
    signal.addEventListener("abort", onAbort, { once: true })
  })
}
