interface BrandMarkProps {
  alt?: string
  decorative?: boolean
  size?: "sm" | "md" | "lg"
  className?: string
}

const sizes = { sm: 32, md: 48, lg: 72 } as const

export function BrandMark({
  alt = "Raindrop",
  decorative = false,
  size = "md",
  className,
}: BrandMarkProps) {
  const pixels = sizes[size]
  return (
    <img
      src="/brand/raindrop-logo-192.png"
      srcSet="/brand/raindrop-logo-32.png 32w, /brand/raindrop-logo-192.png 192w, /brand/raindrop-logo-512.png 512w"
      sizes={`${pixels}px`}
      width={pixels}
      height={pixels}
      alt={decorative ? "" : alt}
      aria-hidden={decorative || undefined}
      className={["raindrop-brand-mark", className].filter(Boolean).join(" ")}
      decoding="async"
    />
  )
}
