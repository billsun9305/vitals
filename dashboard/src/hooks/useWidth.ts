import { useEffect, useState, type RefObject } from 'react'

/**
 * The rendered width of an element, tracked through resizes.
 *
 * Charts draw in pixel coordinates rather than stretching a fixed viewBox,
 * because a stretched viewBox stretches its text too.
 */
export function useWidth(ref: RefObject<HTMLElement | null>): number {
  const [width, setWidth] = useState(0)
  useEffect(() => {
    const el = ref.current
    if (!el) return
    const ro = new ResizeObserver(entries => {
      for (const e of entries) setWidth(e.contentRect.width)
    })
    ro.observe(el)
    setWidth(el.getBoundingClientRect().width)
    return () => ro.disconnect()
  }, [ref])
  return width
}
