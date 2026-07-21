import { useCallback, useEffect, useRef, useState } from "react"

import { ApiClientError } from "../../../shared/api/client"
import {
  lookupTranslation,
  translateEntry,
  translateSelectedText,
} from "../api/translation"
import type {
  LookupResult,
  TranslationResult,
  TranslationTextResult,
} from "../api/translation.generated"

export interface EntryTranslationController {
  entryId: string | null
  result: TranslationResult | null
  lookupResult: LookupResult | null
  selectionResult: TranslationTextResult | null
  isTranslating: boolean
  isLookingUp: boolean
  isTranslatingSelection: boolean
  articleError: TranslationExecutionError | null
  contextError: TranslationExecutionError | null
  translate: () => Promise<boolean>
  lookup: (text: string) => Promise<boolean>
  translateSelection: (text: string) => Promise<boolean>
  clearTranslation: () => void
  clearLookup: () => void
  clearSelectionTranslation: () => void
  cancelContextActions: () => void
  clearError: () => void
}

export type TranslationExecutionError =
  | "TRANSLATE"
  | "LOOKUP"
  | "SELECTION"
  | "DISABLED"
  | "RATE_LIMITED"

export function useEntryTranslationController(
  entryId: string | null,
  csrfToken: string,
  onUnauthenticated: () => void,
): EntryTranslationController {
  const [result, setResult] = useState<TranslationResult | null>(null)
  const [lookupResult, setLookupResult] = useState<LookupResult | null>(null)
  const [selectionResult, setSelectionResult] =
    useState<TranslationTextResult | null>(null)
  const [isTranslating, setIsTranslating] = useState(false)
  const [isLookingUp, setIsLookingUp] = useState(false)
  const [isTranslatingSelection, setIsTranslatingSelection] = useState(false)
  const [articleError, setArticleError] =
    useState<TranslationExecutionError | null>(null)
  const [contextError, setContextError] =
    useState<TranslationExecutionError | null>(null)
  const translateAbort = useRef<AbortController | null>(null)
  const lookupAbort = useRef<AbortController | null>(null)
  const selectionAbort = useRef<AbortController | null>(null)

  useEffect(() => {
    translateAbort.current?.abort()
    lookupAbort.current?.abort()
    selectionAbort.current?.abort()
    translateAbort.current = null
    lookupAbort.current = null
    selectionAbort.current = null
    setIsTranslating(false)
    setIsLookingUp(false)
    setIsTranslatingSelection(false)
    setResult(null)
    setLookupResult(null)
    setSelectionResult(null)
    setArticleError(null)
    setContextError(null)
  }, [entryId])

  useEffect(
    () => () => {
      translateAbort.current?.abort()
      lookupAbort.current?.abort()
      selectionAbort.current?.abort()
    },
    [],
  )

  const translate = useCallback(async () => {
    if (!entryId || isTranslating || isLookingUp || isTranslatingSelection) {
      return false
    }
    const abort = new AbortController()
    translateAbort.current?.abort()
    translateAbort.current = abort
    setIsTranslating(true)
    setArticleError(null)
    try {
      const nextResult = await translateEntry(entryId, csrfToken, abort.signal)
      if (translateAbort.current !== abort) return false
      setResult(nextResult)
      return true
    } catch (cause) {
      if (translateAbort.current !== abort || isAbortError(cause)) return false
      if (isAuthenticationError(cause)) {
        onUnauthenticated()
        return false
      }
      setArticleError(mapExecutionError(cause, "TRANSLATE"))
      return false
    } finally {
      if (translateAbort.current === abort) {
        translateAbort.current = null
        setIsTranslating(false)
      }
    }
  }, [
    csrfToken,
    entryId,
    isLookingUp,
    isTranslating,
    isTranslatingSelection,
    onUnauthenticated,
  ])

  const lookup = useCallback(
    async (text: string) => {
      if (isTranslating || !text.trim()) {
        return false
      }
      const abort = new AbortController()
      lookupAbort.current?.abort()
      selectionAbort.current?.abort()
      lookupAbort.current = abort
      selectionAbort.current = null
      setIsLookingUp(true)
      setIsTranslatingSelection(false)
      setContextError(null)
      setLookupResult(null)
      setSelectionResult(null)
      try {
        const nextResult = await lookupTranslation(text, csrfToken, abort.signal)
        if (lookupAbort.current !== abort) return false
        setLookupResult(nextResult)
        return true
      } catch (cause) {
        if (lookupAbort.current !== abort || isAbortError(cause)) return false
        if (isAuthenticationError(cause)) {
          onUnauthenticated()
          return false
        }
        setContextError(mapExecutionError(cause, "LOOKUP"))
        return false
      } finally {
        if (lookupAbort.current === abort) {
          lookupAbort.current = null
          setIsLookingUp(false)
        }
      }
    },
    [
      csrfToken,
      isTranslating,
      onUnauthenticated,
    ],
  )

  const translateSelection = useCallback(
    async (text: string) => {
      if (
        isTranslating ||
        !text.trim()
      ) {
        return false
      }
      const abort = new AbortController()
      selectionAbort.current?.abort()
      lookupAbort.current?.abort()
      selectionAbort.current = abort
      lookupAbort.current = null
      setIsTranslatingSelection(true)
      setIsLookingUp(false)
      setContextError(null)
      setLookupResult(null)
      setSelectionResult(null)
      try {
        const nextResult = await translateSelectedText(
          text,
          csrfToken,
          abort.signal,
        )
        if (selectionAbort.current !== abort) return false
        setSelectionResult(nextResult)
        return true
      } catch (cause) {
        if (selectionAbort.current !== abort || isAbortError(cause)) return false
        if (isAuthenticationError(cause)) {
          onUnauthenticated()
          return false
        }
        setContextError(mapExecutionError(cause, "SELECTION"))
        return false
      } finally {
        if (selectionAbort.current === abort) {
          selectionAbort.current = null
          setIsTranslatingSelection(false)
        }
      }
    },
    [
      csrfToken,
      isTranslating,
      onUnauthenticated,
    ],
  )

  return {
    entryId,
    result,
    lookupResult,
    selectionResult,
    isTranslating,
    isLookingUp,
    isTranslatingSelection,
    articleError,
    contextError,
    translate,
    lookup,
    translateSelection,
    clearTranslation: useCallback(() => setResult(null), []),
    clearLookup: useCallback(() => setLookupResult(null), []),
    clearSelectionTranslation: useCallback(
      () => setSelectionResult(null),
      [],
    ),
    cancelContextActions: useCallback(() => {
      lookupAbort.current?.abort()
      selectionAbort.current?.abort()
      lookupAbort.current = null
      selectionAbort.current = null
      setIsLookingUp(false)
      setIsTranslatingSelection(false)
      setLookupResult(null)
      setSelectionResult(null)
      setContextError(null)
    }, []),
    clearError: useCallback(() => {
      setArticleError(null)
      setContextError(null)
    }, []),
  }
}

function mapExecutionError(
  cause: unknown,
  fallback: "TRANSLATE" | "LOOKUP" | "SELECTION",
): TranslationExecutionError {
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
