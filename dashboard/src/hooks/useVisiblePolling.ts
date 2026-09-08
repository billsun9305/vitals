import { useEffect, useRef } from 'react'

/**
 * Run `callback` on a fixed interval, but only while the tab is visible.
 *
 * This is the whole reason the dashboard costs nothing when the tab is
 * closed or backgrounded: the interval is *stopped* on `visibilitychange`
 * (not merely skipped inside a still-running tick), and started again —
 * with an immediate fetch, so switching back doesn't wait a full period —
 * when the tab becomes visible. No websocket, no SSE, no connection that
 * keeps a socket (and the OS resources behind it) alive while nobody is
 * looking.
 *
 * `callback` is read through a ref so callers can pass a fresh closure each
 * render without tearing down and rebuilding the interval every time.
 */
export function useVisiblePolling(callback: () => void, intervalMs: number): void {
  const callbackRef = useRef(callback)
  useEffect(() => {
    callbackRef.current = callback
  }, [callback])

  useEffect(() => {
    let id: number | undefined

    const tick = () => callbackRef.current()

    const stop = () => {
      if (id !== undefined) {
        window.clearInterval(id)
        id = undefined
      }
    }

    const start = () => {
      if (id !== undefined) return
      tick()
      id = window.setInterval(tick, intervalMs)
    }

    const onVisibilityChange = () => {
      if (document.visibilityState === 'visible') {
        start()
      } else {
        stop()
      }
    }

    if (document.visibilityState === 'visible') {
      start()
    }
    document.addEventListener('visibilitychange', onVisibilityChange)

    return () => {
      stop()
      document.removeEventListener('visibilitychange', onVisibilityChange)
    }
  }, [intervalMs])
}
