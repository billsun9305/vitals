import type { Core } from '../types'

/** One bar per core, E cores then P cores, colored by kind. */
export function CoreBars({ cores }: { cores: Core[] }) {
  if (cores.length === 0) return null
  return (
    <div className="core-bars" role="img" aria-label="per-core utilization">
      {cores.map(c => (
        <div
          key={c.id}
          className={`core-bar core-bar--${c.kind}`}
          title={`${c.kind}${c.core_id} — ${c.pct.toFixed(1)}% @ ${c.freq_mhz} MHz`}
          style={{ height: `${Math.max(c.pct, 2)}%` }}
        />
      ))}
    </div>
  )
}
