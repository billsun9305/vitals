import type { ReactNode } from 'react'
import type { Level } from '../format'
import { Sparkline } from './Sparkline'

interface Props {
  label: string
  value: string
  sub?: string
  /** A status dot beside the label; omitted when the tile has no state. */
  level?: Level
  trend?: number[]
  trendLabel?: string
  children?: ReactNode
}

const DOT: Record<Level, string> = {
  nominal: 'dot--good',
  warning: 'dot--warning',
  critical: 'dot--critical',
}

/** Stat tile: label, headline value, one line of context, a trend. */
export function StatTile({ label, value, sub, level, trend, trendLabel, children }: Props) {
  return (
    <div className="tile">
      <div className="tile-label">
        {level && <span className={`dot ${DOT[level]}`} aria-hidden="true" />}
        {label}
      </div>
      <div className="tile-value">{value}</div>
      {sub && <div className="tile-sub">{sub}</div>}
      {trend && <Sparkline values={trend} label={trendLabel ?? `${label} trend`} />}
      {children}
    </div>
  )
}
