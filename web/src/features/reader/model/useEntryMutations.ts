import { useCallback, useRef } from "react"

import { invalidResponseError } from "../../../shared/api/client"
import type { ReaderApi } from "./controllerApi"
import type { ReaderSession } from "./controllerSession"
import {
  isAbortError,
  isUnauthenticatedError,
  readerErrorMessage,
} from "./controllerErrors"
import type { ReaderAction } from "./reducer"
import type { EntryMutationField, ReaderState } from "./types"

interface EntryMutationOptions {
  api: ReaderApi
  csrfToken: string
  dispatch: (action: ReaderAction) => void
  stateRef: { current: ReaderState }
  session: ReaderSession
  onMutationSettled: (field: EntryMutationField) => void
  runReadAction: <T>(operation: () => Promise<T>) => Promise<T>
}

export function useEntryMutations({
  api,
  csrfToken,
  dispatch,
  stateRef,
  session,
  onMutationSettled,
  runReadAction,
}: EntryMutationOptions) {
  const nextMutationId = useRef(0)

  const executeToggle = useCallback(
    async (entryId: string, field: EntryMutationField) => {
      const entity =
        stateRef.current.entriesById[entryId] ?? stateRef.current.detailsById[entryId]
      if (!entity) return
      const task = session.begin()
      if (!task) return
      const mutationId = ++nextMutationId.current
      const value = !entity[field]
      let shouldRevalidate = false
      dispatch({ type: "entryMutationStarted", mutationId, entryId, field, value })
      try {
        const request = field === "isRead" ? { isRead: value } : { isStarred: value }
        const response = await api.patchEntryState(
          entryId,
          request,
          csrfToken,
          task.controller.signal,
        )
        if (!session.isCurrent(task)) return
        if (response.entryId !== entryId) throw invalidResponseError()
        dispatch({ type: "entryMutationSucceeded", mutationId, state: response })
        shouldRevalidate = true
      } catch (error) {
        if (isAbortError(error)) return
        if (!session.isCurrent(task)) return
        if (isUnauthenticatedError(error)) return session.expire(task)
        dispatch({
          type: "entryMutationFailed",
          mutationId,
          error: readerErrorMessage(error),
        })
        shouldRevalidate = true
      } finally {
        session.finish(task)
        if (shouldRevalidate && session.active()) onMutationSettled(field)
      }
    },
    [api, csrfToken, dispatch, onMutationSettled, session, stateRef],
  )
  const toggleField = useCallback(
    (entryId: string, field: EntryMutationField) => {
      const execute = () => executeToggle(entryId, field)
      return field === "isRead" ? runReadAction(execute) : execute()
    },
    [executeToggle, runReadAction],
  )

  return {
    toggleRead: (entryId: string) => toggleField(entryId, "isRead"),
    toggleStar: (entryId: string) => toggleField(entryId, "isStarred"),
  }
}
