import { useCallback, useEffect, useRef } from "react"

import { invalidResponseError } from "../../../shared/api/client"
import type { ReaderApi } from "./controllerApi"
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
  expireSession: () => void
}

export function useEntryMutations({
  api,
  csrfToken,
  dispatch,
  stateRef,
  expireSession,
}: EntryMutationOptions) {
  const controllers = useRef(new Set<AbortController>())
  const nextMutationId = useRef(0)

  const toggleField = useCallback(
    async (entryId: string, field: EntryMutationField) => {
      const entity =
        stateRef.current.entriesById[entryId] ?? stateRef.current.detailsById[entryId]
      if (!entity) return
      const mutationId = ++nextMutationId.current
      const value = !entity[field]
      const controller = new AbortController()
      controllers.current.add(controller)
      dispatch({ type: "entryMutationStarted", mutationId, entryId, field, value })
      try {
        const request = field === "isRead" ? { isRead: value } : { isStarred: value }
        const response = await api.patchEntryState(
          entryId,
          request,
          csrfToken,
          controller.signal,
        )
        if (response.entryId !== entryId) throw invalidResponseError()
        dispatch({ type: "entryMutationSucceeded", mutationId, state: response })
      } catch (error) {
        if (isAbortError(error)) return
        if (isUnauthenticatedError(error)) return expireSession()
        dispatch({
          type: "entryMutationFailed",
          mutationId,
          error: readerErrorMessage(error),
        })
      } finally {
        controllers.current.delete(controller)
      }
    },
    [api, csrfToken, dispatch, expireSession, stateRef],
  )

  useEffect(
    () => () => {
      for (const controller of controllers.current) controller.abort()
    },
    [],
  )

  return {
    toggleRead: (entryId: string) => toggleField(entryId, "isRead"),
    toggleStar: (entryId: string) => toggleField(entryId, "isStarred"),
  }
}
