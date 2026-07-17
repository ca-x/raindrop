import { useCallback, useEffect, useMemo, useRef } from "react"

import type { ReaderAction } from "./reducer"

export interface SessionTask {
  controller: AbortController
  epoch: number
}

export interface ReaderSession {
  active: () => boolean
  begin: () => SessionTask | null
  isCurrent: (task: SessionTask) => boolean
  finish: (task: SessionTask) => void
  expire: (task: SessionTask) => void
}

interface SessionState {
  epoch: number
  expired: boolean
  controllers: Set<AbortController>
}

export function useReaderSession(
  dispatch: (action: ReaderAction) => void,
  onUnauthenticated: () => void,
): ReaderSession {
  const sessionRef = useRef<SessionState>({
    epoch: 0,
    expired: false,
    controllers: new Set(),
  })
  const active = useCallback(() => !sessionRef.current.expired, [])
  const begin = useCallback((): SessionTask | null => {
    const session = sessionRef.current
    if (session.expired) return null
    const controller = new AbortController()
    session.controllers.add(controller)
    return { controller, epoch: session.epoch }
  }, [])
  const isCurrent = useCallback(
    (task: SessionTask) =>
      !sessionRef.current.expired && sessionRef.current.epoch === task.epoch,
    [],
  )
  const finish = useCallback((task: SessionTask) => {
    sessionRef.current.controllers.delete(task.controller)
  }, [])
  const expire = useCallback(
    (task: SessionTask) => {
      const session = sessionRef.current
      if (session.expired || session.epoch !== task.epoch) return
      session.expired = true
      session.epoch += 1
      for (const controller of session.controllers) controller.abort()
      session.controllers.clear()
      dispatch({ type: "sessionExpired" })
      onUnauthenticated()
    },
    [dispatch, onUnauthenticated],
  )

  useEffect(
    () => {
      sessionRef.current.expired = false
      return () => {
        const session = sessionRef.current
        session.expired = true
        session.epoch += 1
        for (const controller of session.controllers) controller.abort()
        session.controllers.clear()
      }
    },
    [],
  )

  return useMemo(
    () => ({ active, begin, isCurrent, finish, expire }),
    [active, begin, expire, finish, isCurrent],
  )
}
