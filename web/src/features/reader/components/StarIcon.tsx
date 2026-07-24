interface StarIconProps {
  isFilled?: boolean
}

export function StarIcon({ isFilled = false }: StarIconProps) {
  return (
    <svg
      aria-hidden="true"
      focusable="false"
      viewBox="0 0 20 20"
      width="18"
      height="18"
      fill={isFilled ? "currentColor" : "none"}
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinejoin="round"
    >
      <path d="m10 2.75 2.2 4.46 4.92.72-3.56 3.47.84 4.9L10 14l-4.4 2.3.84-4.9-3.56-3.47 4.92-.72L10 2.75Z" />
    </svg>
  )
}
