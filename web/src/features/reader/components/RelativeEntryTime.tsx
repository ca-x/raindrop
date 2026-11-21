import { createContext, useContext, useEffect, useState, type ReactNode } from "react"

const MinuteClock = createContext<number | null>(null)
const formatters = new Map<string, { absolute: Intl.DateTimeFormat; relative: Intl.RelativeTimeFormat }>()

function timeFormats(locale: string) {
  let formats = formatters.get(locale)
  if (!formats) {
    formats = {
      absolute: new Intl.DateTimeFormat(locale, { dateStyle: "medium", timeStyle: "short" }),
      relative: new Intl.RelativeTimeFormat(locale, { numeric: "always" }),
    }
    formatters.set(locale, formats)
  }
  return formats
}

interface RelativeEntryTimeProps {
  timestampUs: number
  locale: string
  justNowLabel: string
}

export function RelativeEntryTime({
  timestampUs,
  locale,
  justNowLabel,
}: RelativeEntryTimeProps) {
  const nowMs = useContext(MinuteClock) ?? Date.now()
  const date = new Date(timestampUs / 1000)
  return (
    <time
      dateTime={date.toISOString()}
      title={timeFormats(locale).absolute.format(date)}
    >
      {formatRelativeEntryTime(date.getTime(), nowMs, locale, justNowLabel)}
    </time>
  )
}

export function formatRelativeEntryTime(
  timestampMs: number,
  nowMs: number,
  locale: string,
  justNowLabel: string,
): string {
  const differenceMs = timestampMs - nowMs
  const absoluteMs = Math.abs(differenceMs)
  if (absoluteMs < 60_000) return justNowLabel

  const [divisor, unit] = absoluteMs < 60 * 60_000
    ? [60_000, "minute"]
    : absoluteMs < 24 * 60 * 60_000
      ? [60 * 60_000, "hour"]
      : absoluteMs < 30 * 24 * 60 * 60_000
        ? [24 * 60 * 60_000, "day"]
        : absoluteMs < 365 * 24 * 60 * 60_000
          ? [30 * 24 * 60 * 60_000, "month"]
          : [365 * 24 * 60 * 60_000, "year"]
  const value = Math.round(differenceMs / divisor) || (differenceMs < 0 ? -1 : 1)
  return timeFormats(locale).relative.format(
    value,
    unit as Intl.RelativeTimeFormatUnit,
  )
}

export function ReaderClock({ children }: { children: ReactNode }) {
  const [nowMs, setNowMs] = useState(() => Date.now())
  useEffect(() => {
    let timeout: ReturnType<typeof setTimeout>
    const tick = () => {
      clearTimeout(timeout)
      if (document.hidden) return
      const now = Date.now()
      setNowMs(now)
      timeout = setTimeout(tick, 60_000 - (now % 60_000))
    }
    tick()
    document.addEventListener("visibilitychange", tick)
    return () => {
      clearTimeout(timeout)
      document.removeEventListener("visibilitychange", tick)
    }
  }, [])
  return <MinuteClock value={nowMs}>{children}</MinuteClock>
}
