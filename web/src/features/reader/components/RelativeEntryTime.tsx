import { useEffect, useState } from "react"

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
  const nowMs = useMinuteClock()
  const date = new Date(timestampUs / 1000)
  return (
    <time
      dateTime={date.toISOString()}
      title={new Intl.DateTimeFormat(locale, {
        dateStyle: "medium",
        timeStyle: "short",
      }).format(date)}
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
  return new Intl.RelativeTimeFormat(locale, { numeric: "always" }).format(
    value,
    unit as Intl.RelativeTimeFormatUnit,
  )
}

function useMinuteClock(): number {
  const [nowMs, setNowMs] = useState(() => Date.now())
  useEffect(() => {
    let interval: ReturnType<typeof setInterval> | undefined
    const timeout = setTimeout(() => {
      setNowMs(Date.now())
      interval = setInterval(() => setNowMs(Date.now()), 60_000)
    }, 60_000 - (Date.now() % 60_000))
    return () => {
      clearTimeout(timeout)
      if (interval) clearInterval(interval)
    }
  }, [])
  return nowMs
}
