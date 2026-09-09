import { useState } from 'react'
import { pct } from '../format'
import type { ProcRow, TopReport } from '../types'

type Order = 'cpu' | 'mem'

function Bar({ fraction }: { fraction: number }) {
  return (
    <i aria-hidden="true">
      <b style={{ width: `${Math.min(Math.max(fraction, 0), 1) * 100}%` }} />
    </i>
  )
}

/**
 * Top processes, ranked by CPU or by memory. The inline bar beside the
 * ranked column is relative to the largest row shown, so the eye gets the
 * shape of the list without reading every number.
 */
export function ProcessTable({ top }: { top: TopReport }) {
  const [order, setOrder] = useState<Order>('cpu')
  const rows: ProcRow[] = order === 'cpu' ? top.by_cpu : top.by_mem
  const maxCpu = Math.max(1, ...rows.map(r => r.cpu_pct))
  const maxMem = Math.max(1, ...rows.map(r => r.mem_mb))

  return (
    <section className="card">
      <header className="card-head">
        <div className="card-title">
          <h2>Processes</h2>
          <span className="card-sub">top {rows.length}</span>
        </div>
        <div className="seg" role="group" aria-label="Rank processes by">
          <button aria-pressed={order === 'cpu'} onClick={() => setOrder('cpu')}>
            By CPU
          </button>
          <button aria-pressed={order === 'mem'} onClick={() => setOrder('mem')}>
            By memory
          </button>
        </div>
      </header>
      <table>
        <thead>
          <tr>
            <th>Process</th>
            <th className="num">CPU</th>
            <th className="num">Memory</th>
            <th className="num">Threads</th>
          </tr>
        </thead>
        <tbody>
          {rows.map(p => (
            <tr key={p.pid}>
              <td>
                <div className="proc-name">
                  <strong>{p.app ?? p.name}</strong>
                  <span>
                    {p.pid}
                    {p.app && p.app !== p.name ? ` · ${p.name}` : ''}
                  </span>
                </div>
              </td>
              <td className="num">
                <span className="inline-bar">
                  {order === 'cpu' && <Bar fraction={p.cpu_pct / maxCpu} />}
                  {pct(p.cpu_pct)}
                </span>
              </td>
              <td className="num">
                <span className="inline-bar">
                  {order === 'mem' && <Bar fraction={p.mem_mb / maxMem} />}
                  {p.mem_mb.toLocaleString()} MB
                </span>
              </td>
              <td className="num">{p.threads}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  )
}
