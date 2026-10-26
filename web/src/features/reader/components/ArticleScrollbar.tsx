import { useEffect, useRef, type KeyboardEvent, type PointerEvent, type RefObject } from "react"

interface ArticleScrollbarProps {
  articleRef: RefObject<HTMLElement | null>
  bodyRef: RefObject<HTMLDivElement | null>
  articleId: string
  entryId: string | null
  label: string
}

const radius = 32
const inset = 8.5
const innerRadius = radius - inset
const trail = 20
const thumbSize = 70

export function ArticleScrollbar(props: ArticleScrollbarProps) {
  const barRef = useRef<HTMLDivElement>(null)
  const svgRef = useRef<SVGSVGElement>(null)
  const syncRef = useRef(() => {})
  const dragRef = useRef<{ pointerId: number; clientY: number; scrollTop: number } | null>(null)

  useEffect(() => {
    const article = props.articleRef.current
    const bar = barRef.current
    const svg = svgRef.current
    if (!article || !bar || !svg || !props.entryId) return
    const thumbs = svg.querySelectorAll<SVGPathElement>(".reader-article-scrollbar-thumb")
    let previousHeight = -1
    let length = 0
    const corner = Math.ceil(trail + Math.PI * innerRadius / 2)
    const sync = () => {
      const height = article.clientHeight
      const distance = Math.max(0, article.scrollHeight - height)
      const progress = distance === 0 ? 0 : clamp(article.scrollTop / distance)
      bar.hidden = distance <= 1 || height < radius * 2
      bar.setAttribute("aria-valuenow", String(Math.round(progress * 100)))
      if (height !== previousHeight) {
        previousHeight = height
        const path = [
          `M ${radius - trail} ${inset}`,
          `H ${radius}`,
          `a ${innerRadius} ${innerRadius} 0 0 1 ${innerRadius} ${innerRadius}`,
          `V ${Math.max(radius, height - radius)}`,
          `a ${innerRadius} ${innerRadius} 0 0 1 ${-innerRadius} ${innerRadius}`,
          `H ${radius - trail}`,
        ].join(" ")
        svg.setAttribute("viewBox", `0 0 ${radius * 2} ${Math.max(height, radius * 2)}`)
        for (const segment of svg.querySelectorAll("path")) segment.setAttribute("d", path)
        length = trail * 2 + Math.PI * innerRadius + Math.max(0, height - radius * 2)
      }
      const padding = Math.min(0.15, 60 / Math.max(1, distance))
      const start = corner - 50
      const end = length - corner - thumbSize + 50
      const offset = progress < padding
        ? interpolate(35, -start, progress / padding)
        : progress > 1 - padding
          ? interpolate(-end, -(length - 35), (progress - 1 + padding) / padding)
          : interpolate(-start, -end, (progress - padding) / (1 - padding * 2))
      for (const thumb of thumbs) {
        thumb.setAttribute("stroke-dasharray", `${thumbSize} ${length}`)
        thumb.setAttribute("stroke-dashoffset", String(offset))
      }
    }
    const resize = () => {
      const width = article.parentElement?.clientWidth ?? article.clientWidth
      bar.style.insetInlineEnd = `${Math.max(0, (width - article.clientWidth) / 2)}px`
      sync()
    }
    const wheel = (event: WheelEvent) => {
      if (event.ctrlKey) return
      const unit = event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? article.clientHeight : 1
      event.preventDefault()
      article.scrollTop += event.deltaY * unit
      sync()
    }
    syncRef.current = sync
    resize()
    const observer = new ResizeObserver(resize)
    observer.observe(article)
    if (article.parentElement) observer.observe(article.parentElement)
    if (props.bodyRef.current) observer.observe(props.bodyRef.current)
    article.addEventListener("scroll", sync, { passive: true })
    bar.addEventListener("wheel", wheel, { passive: false })
    return () => {
      observer.disconnect()
      article.removeEventListener("scroll", sync)
      bar.removeEventListener("wheel", wheel)
      dragRef.current = null
      delete bar.dataset.dragging
      bar.hidden = true
      syncRef.current = () => {}
    }
  }, [props.articleRef, props.bodyRef, props.entryId])

  const scrollTo = (offset: number) => {
    const article = props.articleRef.current
    if (!article) return
    article.scrollTop = Math.max(0, Math.min(offset, article.scrollHeight - article.clientHeight))
    syncRef.current()
  }

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const article = props.articleRef.current
    if (!article) return
    const page = article.clientHeight * 0.9
    const offsets: Record<string, number> = {
      ArrowUp: article.scrollTop - 40,
      ArrowDown: article.scrollTop + 40,
      PageUp: article.scrollTop - page,
      PageDown: article.scrollTop + page,
      Home: 0,
      End: article.scrollHeight,
      " ": article.scrollTop + (event.shiftKey ? -page : page),
    }
    const offset = offsets[event.key]
    if (offset === undefined) return
    event.preventDefault()
    event.stopPropagation()
    scrollTo(offset)
  }

  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    const article = props.articleRef.current
    const svg = svgRef.current
    if (!article || !svg || dragRef.current || event.button !== 0) return
    event.preventDefault()
    event.currentTarget.focus({ preventScroll: true })
    event.currentTarget.setPointerCapture(event.pointerId)
    if (!(event.target as Element).classList.contains("reader-article-scrollbar-thumb")) {
      const bounds = svg.getBoundingClientRect()
      const progress = clamp((event.clientY - bounds.top - inset) / Math.max(1, bounds.height - inset * 2))
      scrollTo(progress * (article.scrollHeight - article.clientHeight))
    }
    dragRef.current = {
      pointerId: event.pointerId,
      clientY: event.clientY,
      scrollTop: article.scrollTop,
    }
    event.currentTarget.dataset.dragging = "true"
  }

  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current
    const article = props.articleRef.current
    if (!drag || !article || event.pointerId !== drag.pointerId) return
    const distance = article.scrollHeight - article.clientHeight
    scrollTo(drag.scrollTop + (event.clientY - drag.clientY) * distance / Math.max(1, article.clientHeight - thumbSize))
  }

  const stopDragging = (event: PointerEvent<HTMLDivElement>) => {
    if (dragRef.current?.pointerId !== event.pointerId) return
    dragRef.current = null
    delete event.currentTarget.dataset.dragging
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId)
    }
  }

  return (
    <div
      ref={barRef}
      className="reader-article-scrollbar"
      role="scrollbar"
      aria-label={props.label}
      aria-controls={props.articleId}
      aria-orientation="vertical"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={0}
      tabIndex={0}
      hidden
      onKeyDown={onKeyDown}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={stopDragging}
      onPointerCancel={stopDragging}
      onLostPointerCapture={stopDragging}
    >
      <svg ref={svgRef} fill="none" strokeLinecap="round" preserveAspectRatio="none" aria-hidden="true" focusable="false">
        <path className="reader-article-scrollbar-hit" />
        <path className="reader-article-scrollbar-track" />
        <path className="reader-article-scrollbar-thumb" />
        <path className="reader-article-scrollbar-thumb reader-article-scrollbar-hit" />
      </svg>
    </div>
  )
}

function clamp(value: number) {
  return Math.max(0, Math.min(1, value))
}

function interpolate(start: number, end: number, progress: number) {
  return start + (end - start) * progress
}
