import { useCallback, useEffect, useRef, useState } from "react"

import { ApiClientError } from "../../../shared/api/client"
import { lookupTranslation, translateEntry } from "../api/translation"
import type {
  LookupResult,
  TranslationResult,
} from "../api/translation.generated"

export interface EntryTranslationController {
  entryId: string | null
  result: TranslationResult | null
  lookupResult: LookupResult | null
  isTranslating: boolean
  isLookingUp: boolean
  error: "TRANSLATE" | "LOOKUP" | "DISABLED" | "RATE_LIMITED" | null
  translate: () => Promise<boolean>
  lookup: (text: string) => Promise<boolean>
  clearTranslation: () => void
  clearLookup: () => void
  clearError: () => void
}

export function useEntryTranslationController(
  entryId: string | null,
  csrfToken: string,
  onUnauthenticated: () => void,
): EntryTranslationController {
  const [result, setResult] = useState<TranslationResult | null>(null)
  const [lookupResult, setLookupResult] = useState<LookupResult | null>(null)
  const [isTranslating, setIsTranslating] = useState(false)
  const [isLookingUp, setIsLookingUp] = useState(false)
  const [error, setError] = useState<EntryTranslationController["error"]>(null)
  const translateAbort = useRef<AbortController | null>(null)
  const lookupAbort = useRef<AbortController | null>(null)

  useEffect(() => {
    translateAbort.current?.abort()
    lookupAbort.current?.abort()
    setResult(null)
    setLookupResult(null)
    setError(null)
  }, [entryId])

  useEffect(
    () => () => {
      translateAbort.current?.abort()
      lookupAbort.current?.abort()
    },
    [],
  )

  const translate = useCallback(async () => {
    if (!entryId || isTranslating) return false
    const abort = new AbortController()
    translateAbort.current?.abort()
    translateAbort.current = abort
    setIsTranslating(true)
    setError(null)
    try {
      setResult(await translateEntry(entryId, csrfToken, abort.signal))
      return true
    } catch (cause) {
      if (isAbortError(cause)) return false
      if (isAuthenticationError(cause)) {
        onUnauthenticated()
        return false
      }
      setError(mapExecutionError(cause, "TRANSLATE"))
      return false
    } finally {
      if (translateAbort.current === abort) translateAbort.current = null
      setIsTranslating(false)
    }
  }, [csrfToken, entryId, isTranslating, onUnauthenticated])

  const lookup = useCallback(
    async (text: string) => {
      if (isLookingUp || !text.trim()) return false
      const abort = new AbortController()
      lookupAbort.current?.abort()
      lookupAbort.current = abort
      setIsLookingUp(true)
      setError(null)
      try {
        setLookupResult(await lookupTranslation(text, csrfToken, abort.signal))
        return true
      } catch (cause) {
        if (isAbortError(cause)) return false
        if (isAuthenticationError(cause)) {
          onUnauthenticated()
          return false
        }
        setError(mapExecutionError(cause, "LOOKUP"))
        return false
      } finally {
        if (lookupAbort.current === abort) lookupAbort.current = null
        setIsLookingUp(false)
      }
    },
    [csrfToken, isLookingUp, onUnauthenticated],
  )

  return {
    entryId,
    result,
    lookupResult,
    isTranslating,
    isLookingUp,
    error,
    translate,
    lookup,
    clearTranslation: useCallback(() => setResult(null), []),
    clearLookup: useCallback(() => setLookupResult(null), []),
    clearError: useCallback(() => setError(null), []),
  }
}

function mapExecutionError(
  cause: unknown,
  fallback: "TRANSLATE" | "LOOKUP",
): EntryTranslationController["error"] {
  if (!(cause instanceof ApiClientError)) return fallback
  if (
    cause.payload.code === "TRANSLATION_DISABLED" ||
    cause.payload.code === "TRANSLATION_NOT_CONFIGURED" ||
    cause.payload.code === "TRANSLATION_PROVIDER_UNAVAILABLE"
  ) {
    return "DISABLED"
  }
  if (cause.status === 429) return "RATE_LIMITED"
  return fallback
}

function isAuthenticationError(error: unknown): boolean {
  return error instanceof ApiClientError && error.status === 401
}

function isAbortError(error: unknown): boolean {
  return error instanceof DOMException && error.name === "AbortError"
}
