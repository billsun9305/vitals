import { useState, type ReactNode } from 'react'

interface Props {
  title: string
  value?: string
  sub?: string
  legend?: ReactNode
  /** The chart's table twin. When given, a Chart / Table toggle appears. */
  table?: () => ReactNode
  children: ReactNode
}

/**
 * The container every chart mounts in: a header row (title, headline value,
 * legend) and the table-view toggle that is each chart's accessibility twin.
 */
export function ChartCard({ title, value, sub, legend, table, children }: Props) {
  const [view, setView] = useState<'chart' | 'table'>('chart')
  return (
    <section className="card">
      <header className="card-head">
        <div className="card-title">
          <h2>{title}</h2>
          {value && <span className="card-value">{value}</span>}
          {sub && <span className="card-sub">{sub}</span>}
        </div>
        <div className="card-tools">
          {legend && <div className="legend">{legend}</div>}
          {table && (
            <div className="seg" role="group" aria-label={`${title} view`}>
              <button aria-pressed={view === 'chart'} onClick={() => setView('chart')}>
                Chart
              </button>
              <button aria-pressed={view === 'table'} onClick={() => setView('table')}>
                Table
              </button>
            </div>
          )}
        </div>
      </header>
      {view === 'table' && table ? <div className="card-table">{table()}</div> : children}
    </section>
  )
}
