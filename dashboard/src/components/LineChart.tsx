import { useId, useMemo, useRef, useState, type KeyboardEvent, type PointerEvent } from 'react'
import { useWidth } from '../hooks/useWidth'
import { ago, clock } from '../format'

export interface Point {
  t: number
  v: number
}

interface Props {
  points: Point[]
  /** Seconds of history to show; the x-axis spans exactly this. */
  windowSec: number
  height?: number
  max?: number
  label: string
  format: (v: number) => string
}

const PAD = { top: 12, right: 60, bottom: 24, left: 40 }
const TICKS = [0, 25, 50, 75, 100]

/**
 * A single-series line over time with the interaction layer built in: a
 * crosshair that snaps to the nearest sample, a tooltip, keyboard stepping,
 * and one direct label at the endpoint. Percent scale, fixed 0–100, so two of
 * these side by side are always comparable.
 */
export function LineChart({ points, windowSec, height = 224, max = 100, label, format }: Props) {
  const ref = useRef<HTMLDivElement>(null)
  const width = useWidth(ref)
  const [hover, setHover] = useState<number | null>(null)
  const clipId = useId()

  const now = points.length ? points[points.length - 1].t : 0
  const t0 = now - windowSec * 1000
  const visible = useMemo(() => points.filter(p => p.t >= t0), [points, t0])

  const plotW = Math.max(width - PAD.left - PAD.right, 0)
  const plotH = height - PAD.top - PAD.bottom
  const baseline = PAD.top + plotH
  const x = (t: number) => PAD.left + ((t - t0) / (windowSec * 1000)) * plotW
  const y = (v: number) => PAD.top + plotH - (Math.min(Math.max(v, 0), max) / max) * plotH

  const line = visible
    .map((p, i) => `${i ? 'L' : 'M'}${x(p.t).toFixed(1)},${y(p.v).toFixed(1)}`)
    .join(' ')
  const area =
    visible.length > 1
      ? `${line} L${x(visible[visible.length - 1].t).toFixed(1)},${baseline} L${x(visible[0].t).toFixed(1)},${baseline} Z`
      : ''
  const last = visible[visible.length - 1]
  const hp = hover != null ? visible[hover] : undefined

  function nearest(px: number): number {
    let best = 0
    let bestD = Infinity
    for (let i = 0; i < visible.length; i++) {
      const d = Math.abs(x(visible[i].t) - px)
      if (d < bestD) {
        bestD = d
        best = i
      }
    }
    return best
  }

  function onMove(e: PointerEvent<SVGSVGElement>) {
    if (!visible.length) return
    const rect = e.currentTarget.getBoundingClientRect()
    setHover(nearest(e.clientX - rect.left))
  }

  function onKey(e: KeyboardEvent<SVGSVGElement>) {
    if (!visible.length) return
    const cur = hover ?? visible.length - 1
    if (e.key === 'ArrowLeft') setHover(Math.max(0, cur - 1))
    else if (e.key === 'ArrowRight') setHover(Math.min(visible.length - 1, cur + 1))
    else if (e.key === 'Escape') setHover(null)
    else return
    e.preventDefault()
  }

  // Ticks land on round seconds (every 30 s, 1 min or 3 min) rather than on
  // fifths of the window, so a 15-minute axis never reads "−11.3m".
  const step = windowSec <= 120 ? 30 : windowSec / 5
  const xLabels: { x: number; text: string }[] = []
  for (let s = windowSec; s >= 0; s -= step) {
    xLabels.push({ x: PAD.left + (1 - s / windowSec) * plotW, text: ago(s) })
  }

  return (
    <div className="chart" ref={ref}>
      {width > 0 && (
        <svg
          width={width}
          height={height}
          role="img"
          aria-label={label}
          tabIndex={0}
          onPointerMove={onMove}
          onPointerLeave={() => setHover(null)}
          onFocus={() => setHover(visible.length ? visible.length - 1 : null)}
          onBlur={() => setHover(null)}
          onKeyDown={onKey}
        >
          <clipPath id={clipId}>
            <rect x={PAD.left} y={PAD.top} width={plotW} height={plotH} />
          </clipPath>
          {TICKS.map(tk => (
            <g key={tk}>
              <line
                x1={PAD.left}
                x2={PAD.left + plotW}
                y1={y(tk)}
                y2={y(tk)}
                className={tk === 0 ? 'axis' : 'grid'}
              />
              <text
                x={PAD.left - 8}
                y={y(tk)}
                className="tick"
                textAnchor="end"
                dominantBaseline="middle"
              >
                {tk}%
              </text>
            </g>
          ))}
          {xLabels.map((l, i) => (
            <text
              key={l.text}
              x={l.x}
              y={height - 6}
              className="tick"
              textAnchor={i === 0 ? 'start' : i === xLabels.length - 1 ? 'end' : 'middle'}
            >
              {l.text}
            </text>
          ))}
          {visible.length > 1 ? (
            <g clipPath={`url(#${clipId})`}>
              <path d={area} className="area" />
              <path d={line} className="line" />
            </g>
          ) : (
            <text
              x={PAD.left + plotW / 2}
              y={PAD.top + plotH / 2}
              className="empty"
              textAnchor="middle"
            >
              collecting…
            </text>
          )}
          {hp && (
            <line x1={x(hp.t)} x2={x(hp.t)} y1={PAD.top} y2={baseline} className="crosshair" />
          )}
          {last && (
            <g>
              <circle cx={x(last.t)} cy={y(last.v)} r={6} className="ring" />
              <circle cx={x(last.t)} cy={y(last.v)} r={4} className="dot-mark" />
              <text
                x={x(last.t) + 10}
                y={y(last.v)}
                className="endlabel"
                dominantBaseline="middle"
              >
                {format(last.v)}
              </text>
            </g>
          )}
          {hp && hp !== last && (
            <g>
              <circle cx={x(hp.t)} cy={y(hp.v)} r={6} className="ring" />
              <circle cx={x(hp.t)} cy={y(hp.v)} r={4} className="dot-mark" />
            </g>
          )}
        </svg>
      )}
      {hp && (
        <div
          className="tip"
          style={{
            left: Math.min(Math.max(x(hp.t), 48), Math.max(width - 48, 48)),
            top: y(hp.v),
          }}
        >
          <strong>{format(hp.v)}</strong>
          <span>{clock(hp.t)}</span>
        </div>
      )}
    </div>
  )
}

/** The table twin of a `LineChart`: newest first, so the live value leads. */
export function LineTable({ points, format }: { points: Point[]; format: (v: number) => string }) {
  const rows = points.slice(-120).reverse()
  return (
    <table>
      <thead>
        <tr>
          <th>Time</th>
          <th className="num">Value</th>
        </tr>
      </thead>
      <tbody>
        {rows.map(p => (
          <tr key={p.t}>
            <td>{clock(p.t)}</td>
            <td className="num">{format(p.v)}</td>
          </tr>
        ))}
      </tbody>
    </table>
  )
}
