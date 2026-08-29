type ReaderEmptyIconKind = "sources" | "queue" | "article"

export function ReaderEmptyIcon({ kind }: { kind: ReaderEmptyIconKind }) {
  return (
    <span className="reader-empty-icon">
      <svg
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinecap="round"
        strokeLinejoin="round"
        aria-hidden="true"
      >
        {kind === "sources" ? (
          <>
            <circle cx="6" cy="18" r="1" fill="currentColor" stroke="none" />
            <path d="M5 11a8 8 0 0 1 8 8" />
            <path d="M5 6a13 13 0 0 1 13 13" />
          </>
        ) : kind === "queue" ? (
          <>
            <path d="M7.5 7h11" />
            <path d="M7.5 12h11" />
            <path d="M7.5 17h8" />
            <circle cx="4" cy="7" r=".75" fill="currentColor" stroke="none" />
            <circle cx="4" cy="12" r=".75" fill="currentColor" stroke="none" />
            <circle cx="4" cy="17" r=".75" fill="currentColor" stroke="none" />
          </>
        ) : (
          <>
            <path d="M7 3.5h7l3 3V20a.5.5 0 0 1-.5.5h-9A.5.5 0 0 1 7 20z" />
            <path d="M14 3.5V7h3" />
            <path d="M9.5 11h5" />
            <path d="M9.5 14h5" />
            <path d="M9.5 17h3.5" />
          </>
        )}
      </svg>
    </span>
  )
}
