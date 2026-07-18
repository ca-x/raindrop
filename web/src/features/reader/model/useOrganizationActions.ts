import { useCallback } from "react"

import type { UpdateCategoryRequest } from "../api/organization.generated"
import type { UpdateSubscriptionRequest } from "../api/subscription.generated"
import {
  isAbortError,
  isUnauthenticatedError,
  readerErrorMessage,
} from "./controllerErrors"
import type { ReaderApi } from "./controllerApi"
import type { ReaderSession } from "./controllerSession"
import type { ReaderAction } from "./reducer"

interface OrganizationActionOptions {
  api: ReaderApi
  csrfToken: string
  dispatch: (action: ReaderAction) => void
  session: ReaderSession
}

export function useOrganizationActions({
  api,
  csrfToken,
  dispatch,
  session,
}: OrganizationActionOptions) {
  const runAction = useCallback(
    async <T,>(
      request: (signal: AbortSignal) => Promise<T>,
      success: (value: T) => ReaderAction,
    ): Promise<boolean> => {
      const task = session.begin()
      if (!task) return false
      dispatch({ type: "mutationErrorCleared" })
      try {
        const value = await request(task.controller.signal)
        if (!session.isCurrent(task)) return false
        dispatch(success(value))
        return true
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

  const createCategory = useCallback(
    (title: string) =>
      runAction(
        (signal) => api.createCategory({ title }, csrfToken, signal),
        (category) => ({ type: "categoryUpserted", category }),
      ),
    [api, csrfToken, runAction],
  )

  const updateCategory = useCallback(
    (categoryId: string, request: UpdateCategoryRequest) =>
      runAction(
        (signal) => api.updateCategory(categoryId, request, csrfToken, signal),
        (category) => ({ type: "categoryUpserted", category }),
      ),
    [api, csrfToken, runAction],
  )

  const deleteCategory = useCallback(
    (categoryId: string) =>
      runAction(
        (signal) => api.deleteCategory(categoryId, csrfToken, signal),
        () => ({ type: "categoryDeleted", categoryId }),
      ),
    [api, csrfToken, runAction],
  )

  const updateSubscription = useCallback(
    (subscriptionId: string, request: UpdateSubscriptionRequest) =>
      runAction(
        (signal) => api.updateSubscription(subscriptionId, request, csrfToken, signal),
        (subscription) => ({ type: "subscriptionUpserted", subscription }),
      ),
    [api, csrfToken, runAction],
  )

  return { createCategory, updateCategory, deleteCategory, updateSubscription }
}
