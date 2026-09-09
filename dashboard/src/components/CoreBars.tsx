import { useRef, useState, type PointerEvent } from 'react'
import { useWidth } from '../hooks/useWidth'
import { coreLabels, mhz, pct } from '../format'
import type { Core } from '../types'

const PAD = { top: 12, right: 8, bottom: 22, left: 40 }
const MAX_BAR = 24
const GAP = 2
const RADIUS = 4

/** A column with a rounded data-end and a square base, or a plain rect when too short to round. */
function column(x: number, top: number, w: number, bottom: number): string {
  const h = bottom - top
  if (h < RADIUS || w < RADIUS * 2) {
    return `M${x},${top} h${w} V${bottom} H${x} Z`
  }
  return [
    `M${x},${top + RADIUS}`,
    `a${RADIUS},${RADIUS} 0 0 1 ${RADIUS},-${RADIUS}`,
    `h${w - RADIUS * 2}`,
    `a${RADIUS},${RADIUS} 0 0 1 ${RADIUS},${RADIUS}`,
    `V${bottom} H${x} Z`,
  ].join(' ')
}

/**
 * One column per core, E-cores then P-cores, in the same two colours the
 * menu bar dropdown uses. Bars are capped at 24px and separated by a 2px
 * surface gap; the slot's leftover is air, not bar.
 */
export function CoreBars({ cores, height = 176 }: { cores: Core[]; height?: number }) {
  const ref = useRef<HTMLDivElement>(null)
  const width = useWidth(ref)
  const [hover, setHover] = useState<number | null>(null)

  const n = cores.length
  const plotW = Math.max(width - PAD.left - PAD.right, 0)
  const plotH = height - PAD.top - PAD.bottom
  const baseline = PAD.top + plotH
  const slot = n ? plotW / n : 0
  const barW = Math.max(Math.min(MAX_BAR, slot - GAP), 1)
  const y = (v: number) => PAD.top + plotH - (Math.min(Math.max(v, 0), 100) / 100) * plotH
  const cx = (i: number) => PAD.left + i * slot + slot / 2
  const showLabels = slot >= 18
  const labels = coreLabels(cores)
  const hp = hover != null ? cores[hover] : undefined

  // The whole slot is the hit target, bar and air alike, so a 4px-wide bar
  // at 2% is as easy to inspect as a full one.
  function onMove(e: PointerEvent<SVGSVGElement>) {
    if (!n) return
    const px = e.clientX - e.currentTarget.getBoundingClientRect().left - PAD.left
    setHover(px < 0 || px > plotW ? null : Math.min(n - 1, Math.floor(px / slot)))
  }

  return (
    <div className="chart" ref={ref}>
      {width > 0 && (
        <svg
          width={width}
          height={height}
          role="img"
          aria-label="Per-core CPU utilisation"
          onPointerMove={onMove}
          onPointerLeave={() => setHover(null)}
        >
          {[0, 50, 100].map(tk => (
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
          {cores.map((c, i) => (
            <g key={c.id}>
              <path
                d={column(cx(i) - barW / 2, y(c.pct), barW, baseline)}
                className={`bar bar--${c.kind}${hover === i ? ' is-hover' : ''}`}
              />
              {showLabels && (
                <text x={cx(i)} y={height - 6} className="tick" textAnchor="middle">
                  {labels[i]}
                </text>
              )}
            </g>
          ))}
        </svg>
      )}
      {hp && (
        <div className="tip" style={{ left: cx(hover!), top: y(hp.pct) }}>
          <strong>{pct(hp.pct)}</strong>
          <span>
            {labels[hover!]} · {hp.kind === 'E' ? 'Efficiency' : 'Performance'} core · {mhz(hp.freq_mhz)}
          </span>
        </div>
      )}
    </div>
  )
}

export function CoreTable({ cores }: { cores: Core[] }) {
  const labels = coreLabels(cores)
  return (
    <table>
      <thead>
        <tr>
          <th>Core</th>
          <th>Kind</th>
          <th className="num">Load</th>
          <th className="num">Clock</th>
        </tr>
      </thead>
      <tbody>
        {cores.map((c, i) => (
          <tr key={c.id}>
            <td>{labels[i]}</td>
            <td>{c.kind === 'E' ? 'Efficiency' : 'Performance'}</td>
            <td className="num">{pct(c.pct)}</td>
            <td className="num">{mhz(c.freq_mhz)}</td>
          </tr>
        ))}
      </tbody>
    </table>
  )
}
