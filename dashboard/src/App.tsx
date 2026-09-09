import { useCallback, useState } from 'react'
import './App.css'
import { Sparkline } from './components/Sparkline'
import { CoreBars } from './components/CoreBars'
import { useVisiblePolling } from './hooks/useVisiblePolling'
import type { Snapshot, TopReport, Verdict } from './types'

// Matches the background sampler's own cadence (`crates/app/src/serve/mod.rs`),
// so the dashboard never asks for a sample that doesn't exist yet.
const SNAPSHOT_POLL_MS = 1000
// Process enumeration (`/api/top`) is the most expensive sample the server
// takes, so the dashboard asks for it far less often than the snapshot.
const TOP_AND_PRESSURE_POLL_MS = 5000
const HISTORY_LEN = 120

async function getJSON<T>(path: string): Promise<T> {
  const r = await fetch(path)
  if (!r.ok) throw new Error(`${path} → HTTP ${r.status}`)
  return (await r.json()) as T
}

function fmtUptime(totalSeconds: number): string {
  const d = Math.floor(totalSeconds / 86400)
  const h = Math.floor((totalSeconds % 86400) / 3600)
  const m = Math.floor((totalSeconds % 3600) / 60)
  if (d > 0) return `${d}d ${h}h`
  if (h > 0) return `${h}h ${m}m`
  return `${m}m`
}

export default function App() {
  const [snap, setSnap] = useState<Snapshot | null>(null)
  const [cpuHistory, setCpuHistory] = useState<number[]>([])
  const [gpuHistory, setGpuHistory] = useState<number[]>([])
  const [top, setTop] = useState<TopReport | null>(null)
  const [pressure, setPressure] = useState<Verdict | null>(null)
  const [error, setError] = useState<string | null>(null)

  const pollSnapshot = useCallback(async () => {
    try {
      const s = await getJSON<Snapshot>('/api/snapshot')
      setSnap(s)
      setError(null)
      setCpuHistory(h => [...h, s.cpu_pct].slice(-HISTORY_LEN))
      setGpuHistory(h => [...h, s.gpu_pct].slice(-HISTORY_LEN))
    } catch (e) {
      setError(String(e))
    }
  }, [])

  const pollExtras = useCallback(async () => {
    try {
      const [t, p] = await Promise.all([
        getJSON<TopReport>('/api/top'),
        getJSON<Verdict>('/api/pressure'),
      ])
      setTop(t)
      setPressure(p)
    } catch {
      // Non-fatal: the snapshot poll above already surfaces connectivity
      // trouble, and a stale top/pressure panel is harmless.
    }
  }, [])

  useVisiblePolling(pollSnapshot, SNAPSHOT_POLL_MS)
  useVisiblePolling(pollExtras, TOP_AND_PRESSURE_POLL_MS)

  if (error && !snap) {
    return (
      <main className="state">
        <p>vitals is not responding: {error}</p>
      </main>
    )
  }
  if (!snap) {
    return (
      <main className="state">
        <p>sampling…</p>
      </main>
    )
  }

  return (
    <main>
      <header className="header">
        <div>
          <h1>{snap.host.chip}</h1>
          <p className="muted">
            {snap.host.model} · {snap.host.ecpu_cores}E+{snap.host.pcpu_cores}P CPU ·{' '}
            {snap.host.gpu_cores}-core GPU
          </p>
        </div>
        <div className="badges">
          <span className={`badge badge--${snap.thermal_state}`}>{snap.thermal_state}</span>
          {/* "memory" is load-bearing. This badge is the kernel's memory
              pressure level; the verdict below is the overall health state.
              Labelled just "pressure" they read as a contradiction on screen
              -- a "normal pressure" badge sat directly above "Pressure:
              critical" while swap was 95% full and load was 4.6x cores. */}
          <span className={`badge badge--${snap.mem_pressure}`}>
            memory pressure: {snap.mem_pressure}
          </span>
        </div>
      </header>
      <p className="muted timestamp">
        sampled {snap.sampled_at} · up {fmtUptime(snap.uptime_s)} · load{' '}
        {snap.load_avg.map(l => l.toFixed(2)).join(' ')}
        {error && <span className="warn"> · last poll failed: {error}</span>}
      </p>

      <section className="charts">
        <div className="chart">
          <div className="chart-label">
            <span>CPU</span>
            <span>{snap.cpu_pct.toFixed(1)}%</span>
          </div>
          <Sparkline values={cpuHistory} label="CPU utilization over time" />
        </div>
        <div className="chart">
          <div className="chart-label">
            <span>GPU</span>
            <span>{snap.gpu_pct.toFixed(1)}%</span>
          </div>
          <Sparkline values={gpuHistory} label="GPU utilization over time" />
        </div>
      </section>

      <section>
        <CoreBars cores={snap.cores} />
      </section>

      <section className="stats">
        <dl>
          <dt>CPU (scaled)</dt>
          <dd>{snap.cpu_scaled_pct.toFixed(1)}%</dd>
          <dt>E-cluster</dt>
          <dd>
            {snap.ecpu_pct.toFixed(1)}% @ {snap.ecpu_freq_mhz} MHz
          </dd>
          <dt>P-cluster</dt>
          <dd>
            {snap.pcpu_pct.toFixed(1)}% @ {snap.pcpu_freq_mhz} MHz
          </dd>
          <dt>GPU freq</dt>
          <dd>{snap.gpu_freq_mhz} MHz</dd>
          <dt>Memory</dt>
          <dd>
            {snap.mem_used_mb.toLocaleString()} / {snap.mem_total_mb.toLocaleString()} MB
          </dd>
          <dt>Swap</dt>
          <dd>
            {snap.swap_used_mb.toLocaleString()} / {snap.swap_total_mb.toLocaleString()} MB
          </dd>
          <dt>Power (total)</dt>
          <dd>{snap.power_total_w.toFixed(2)} W</dd>
          <dt>Power (CPU/GPU/ANE)</dt>
          <dd>
            {snap.power_cpu_w.toFixed(2)} / {snap.power_gpu_w.toFixed(2)} /{' '}
            {snap.power_ane_w.toFixed(2)} W
          </dd>
          <dt>CPU temp</dt>
          <dd>{snap.temp_cpu_c.toFixed(1)} °C</dd>
          <dt>GPU temp</dt>
          <dd>{snap.temp_gpu_c.toFixed(1)} °C</dd>
          {snap.fans && snap.fans.length > 0 && (
            <>
              <dt>Fans</dt>
              <dd>{snap.fans.map(f => `${f.name}: ${f.rpm} rpm`).join(', ')}</dd>
            </>
          )}
        </dl>
      </section>

      {pressure && (
        <section className={`pressure pressure--${pressure.state}`}>
          <h2>Overall: {pressure.state}</h2>
          <p>{pressure.summary}</p>
          {pressure.reasons.length > 0 && (
            <ul className="reasons">
              {pressure.reasons.map(r => (
                <li key={r.code} className={`reason--${r.severity}`}>
                  {r.detail}
                </li>
              ))}
            </ul>
          )}
        </section>
      )}

      {top && (
        <section className="top">
          <h2>Top processes by CPU</h2>
          <table>
            <thead>
              <tr>
                <th>PID</th>
                <th>Name</th>
                <th>CPU</th>
                <th>Mem</th>
                <th>Threads</th>
              </tr>
            </thead>
            <tbody>
              {top.by_cpu.map(p => (
                <tr key={p.pid}>
                  <td>{p.pid}</td>
                  <td>{p.app ?? p.name}</td>
                  <td>{p.cpu_pct.toFixed(1)}%</td>
                  <td>{p.mem_mb.toLocaleString()} MB</td>
                  <td>{p.threads}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </section>
      )}
    </main>
  )
}
