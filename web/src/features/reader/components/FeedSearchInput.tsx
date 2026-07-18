import { Icon } from "@astryxdesign/core/Icon"
import { TextInput } from "@astryxdesign/core/TextInput"
import { useLingui } from "@lingui/react"
import { useEffect, useState } from "react"

interface FeedSearchInputProps {
  query: string
  isLoading: boolean
  onSearch: (query: string) => Promise<void>
}

export function FeedSearchInput({ query, isLoading, onSearch }: FeedSearchInputProps) {
  const { i18n } = useLingui()
  const [draft, setDraft] = useState(query)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    setDraft(query)
    setError(null)
  }, [query])

  const submit = async () => {
    const normalized = draft.trim()
    if (new TextEncoder().encode(normalized).length > 128) {
      setError(i18n._("reader.feedSearchTooLong"))
      return
    }
    setError(null)
    await onSearch(normalized)
  }

  return (
    <div className="reader-feed-search">
      <TextInput
        label={i18n._("reader.feedSearch")}
        isLabelHidden
        value={draft}
        placeholder={i18n._("reader.feedSearchPlaceholder")}
        startIcon={<Icon icon="search" />}
        hasClear
        isLoading={isLoading}
        status={error ? { type: "error", message: error } : undefined}
        onChange={(value) => {
          setDraft(value)
          if (error) setError(null)
          if (value === "" && query !== "") void onSearch("")
        }}
        onEnter={() => void submit()}
        width="100%"
      />
    </div>
  )
}
