import { useRef } from 'react'
import { useWidth } from '../hooks/useWidth'

interface Props {
  values: number[]
  max?: number
  height?: number
  label: string
}

/**
 * The trend line inside a stat tile. Drawn in the de-emphasis ink so it reads
 * as context, with only the most recent step in the series colour — the tile's
 * big number is the point; the line says where it came from.
 */
export function Sparkline({ values, max = 100, height = 34, label }: Props) {
  const ref = useRef<HTMLDivElement>(null)
  const width = useWidth(ref)
  const n = values.length
  const step = n > 1 ? width / (n - 1) : 0
  const y = (v: number) => 2 + (height - 4) - (Math.min(Math.max(v, 0), max) / max) * (height - 4)
  const pt = (v: number, i: number) => `${(i * step).toFixed(1)},${y(v).toFixed(1)}`
  const all = values.map(pt).join(' ')
  const live = n > 1 ? `${pt(values[n - 2], n - 2)} ${pt(values[n - 1], n - 1)}` : ''

  return (
    <div className="chart tile-trend" ref={ref}>
      {width > 0 && n > 1 && (
        <svg width={width} height={height} role="img" aria-label={label}>
          <polyline points={all} className="spark-line" />
          <polyline points={live} className="spark-line spark-line--live" />
          <circle cx={(n - 1) * step} cy={y(values[n - 1])} r={5} className="ring" />
          <circle cx={(n - 1) * step} cy={y(values[n - 1])} r={3} className="dot-mark" />
        </svg>
      )}
    </div>
  )
}
