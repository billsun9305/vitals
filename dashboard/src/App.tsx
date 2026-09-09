import { useCallback, useMemo, useState } from 'react'
import './App.css'
import { ChartCard } from './components/ChartCard'
import { CoreBars, CoreTable } from './components/CoreBars'
import { LineChart, LineTable, type Point } from './components/LineChart'
import { Meter } from './components/Meter'
import { ProcessTable } from './components/ProcessTable'
import { RangeControl } from './components/RangeControl'
import { StatTile } from './components/StatTile'
import { Verdict } from './components/Verdict'
import { celsius, clock, gb, levelFor, mhz, pct, uptime, watts, type Level } from './format'
import { useHistory } from './hooks/useHistory'
import type { RangeSec } from './ranges'
import { useVisiblePolling } from './hooks/useVisiblePolling'
import type { PressureLevel, Snapshot, ThermalState, TopReport, Verdict as VerdictData } from './types'

// Matches the background sampler's own cadence (`crates/app/src/serve/mod.rs`),
// so the dashboard never asks for a sample that doesn't exist yet.
const SNAPSHOT_POLL_MS = 1000
// Process enumeration (`/api/top`) is the most expensive sample the server
// takes, so the dashboard asks for it far less often than the snapshot.
const TOP_AND_PRESSURE_POLL_MS = 5000

async function getJSON<T>(path: string): Promise<T> {
  const r = await fetch(path)
  if (!r.ok) throw new Error(`${path} → HTTP ${r.status}`)
  return (await r.json()) as T
}

/** Memory is judged by the kernel's pressure signal, not the used fraction. */
function memLevel(p: PressureLevel): Level {
  return p === 'critical' ? 'critical' : p === 'warning' ? 'warning' : 'nominal'
}

function thermalLevel(t: ThermalState): Level {
  return t === 'critical' || t === 'serious' ? 'critical' : t === 'fair' ? 'warning' : 'nominal'
}

const DOT: Record<Level, string> = { nominal: 'good', warning: 'warning', critical: 'critical' }

export default function App() {
  const [snap, setSnap] = useState<Snapshot | null>(null)
  const [top, setTop] = useState<TopReport | null>(null)
  const [verdict, setVerdict] = useState<VerdictData | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [range, setRange] = useState<RangeSec>(120)
  const { samples, push } = useHistory()

  const pollSnapshot = useCallback(async () => {
    try {
      const s = await getJSON<Snapshot>('/api/snapshot')
      setSnap(s)
      setError(null)
      push({ t: Date.now(), cpu: s.cpu_pct, gpu: s.gpu_pct, memUsedMb: s.mem_used_mb, powerW: s.power_total_w })
    } catch (e) {
      setError(String(e))
    }
  }, [push])

  const pollExtras = useCallback(async () => {
    try {
      const [t, v] = await Promise.all([getJSON<TopReport>('/api/top'), getJSON<VerdictData>('/api/pressure')])
      setTop(t)
      setVerdict(v)
    } catch {
      // The snapshot poll already surfaces connectivity trouble; a stale
      // process list or verdict is harmless for a few seconds.
    }
  }, [])

  useVisiblePolling(pollSnapshot, SNAPSHOT_POLL_MS)
  useVisiblePolling(pollExtras, TOP_AND_PRESSURE_POLL_MS)

  // One slice, applied to everything below the range control, so every chart
  // and tile agrees on what "the last N minutes" means.
  const window = useMemo(() => {
    const newest = samples.length ? samples[samples.length - 1].t : 0
    const t0 = newest - range * 1000
    return samples.filter(s => s.t >= t0)
  }, [samples, range])
  const cpuPoints: Point[] = useMemo(() => window.map(s => ({ t: s.t, v: s.cpu })), [window])
  const gpuPoints: Point[] = useMemo(() => window.map(s => ({ t: s.t, v: s.gpu })), [window])

  if (!snap) {
    return (
      <main className="screen">
        <p>
          <strong>{error ? 'Vitals is not responding' : 'Connecting to vitals…'}</strong>
          {error ? <code>{error}</code> : 'first sample on its way'}
        </p>
      </main>
    )
  }

  const h = snap.host
  const cpuLevel = levelFor(snap.cpu_pct)
  const gpuLevel = levelFor(snap.gpu_pct)
  const mem = memLevel(snap.mem_pressure)
  const memFrac = snap.mem_total_mb ? snap.mem_used_mb / snap.mem_total_mb : 0
  const swapFrac = snap.mem_total_mb ? snap.swap_used_mb / snap.mem_total_mb : 0

  return (
    <main className={`app${error ? ' is-stale' : ''}`}>
      <header className="topbar">
        <div>
          <h1>{h.chip}</h1>
          <p className="sub">
            {h.model} · {h.ecpu_cores}E + {h.pcpu_cores}P CPU · {h.gpu_cores}-core GPU · up {uptime(snap.uptime_s)}
          </p>
        </div>
        <div className="badges">
          <span className="badge">
            <span className={`dot dot--${DOT[thermalLevel(snap.thermal_state)]}`} aria-hidden="true" />
            Thermal {snap.thermal_state}
          </span>
          <span className="badge">
            <span className={`dot dot--${DOT[mem]}`} aria-hidden="true" />
            Memory pressure {snap.mem_pressure}
          </span>
          <span className="meta">
            load {snap.load_avg.map(l => l.toFixed(1)).join(' · ')} · {clock(Date.parse(snap.sampled_at))}
          </span>
        </div>
      </header>

      {verdict && <Verdict v={verdict} />}

      <div className="toolbar">
        <RangeControl value={range} onChange={setRange} />
        {error ? (
          <span className="stale-note">Reconnecting — showing the last good sample · {error}</span>
        ) : (
          <span className="meta">live · 1 s</span>
        )}
      </div>

      <section className="tiles">
        <StatTile
          label="CPU"
          value={pct(snap.cpu_pct)}
          sub={`E ${pct(snap.ecpu_pct, 0)} · P ${pct(snap.pcpu_pct, 0)} · ${mhz(snap.pcpu_freq_mhz)}`}
          level={cpuLevel}
          trend={window.map(s => s.cpu)}
          trendLabel="CPU trend"
        />
        <StatTile
          label="GPU"
          value={pct(snap.gpu_pct)}
          sub={`${mhz(snap.gpu_freq_mhz)} · ${celsius(snap.temp_gpu_c)}`}
          level={gpuLevel}
          trend={window.map(s => s.gpu)}
          trendLabel="GPU trend"
        />
        <StatTile
          label="Memory"
          value={gb(snap.mem_used_mb)}
          sub={`of ${gb(snap.mem_total_mb, 0)} · swap ${gb(snap.swap_used_mb)}`}
          level={mem}
          trend={window.map(s => (snap.mem_total_mb ? (s.memUsedMb / snap.mem_total_mb) * 100 : 0))}
          trendLabel="Memory trend"
        />
        <StatTile
          label="Power"
          value={watts(snap.power_total_w)}
          sub={`CPU ${snap.power_cpu_w.toFixed(2)} · GPU ${snap.power_gpu_w.toFixed(2)} · ANE ${snap.power_ane_w.toFixed(2)} W · ${celsius(snap.temp_cpu_c)}`}
          trend={window.map(s => s.powerW)}
          trendLabel="Power trend"
        />
      </section>

      <section className="grid-2">
        <ChartCard
          title="CPU"
          value={pct(snap.cpu_pct)}
          sub={`scaled ${pct(snap.cpu_scaled_pct)}`}
          table={() => <LineTable points={cpuPoints} format={v => pct(v)} />}
        >
          <LineChart points={cpuPoints} windowSec={range} label="CPU utilisation over time" format={v => pct(v)} />
        </ChartCard>
        <ChartCard
          title="Cores"
          legend={
            <>
              <span>
                <i className="swatch swatch--E" /> E-cores {pct(snap.ecpu_pct, 0)}
              </span>
              <span>
                <i className="swatch swatch--P" /> P-cores {pct(snap.pcpu_pct, 0)}
              </span>
            </>
          }
          table={() => <CoreTable cores={snap.cores} />}
        >
          <CoreBars cores={snap.cores} />
        </ChartCard>
      </section>

      <section className="grid-2">
        <ChartCard
          title="GPU"
          value={pct(snap.gpu_pct)}
          sub={mhz(snap.gpu_freq_mhz)}
          table={() => <LineTable points={gpuPoints} format={v => pct(v)} />}
        >
          <LineChart points={gpuPoints} windowSec={range} label="GPU utilisation over time" format={v => pct(v)} />
        </ChartCard>
        <ChartCard title="Memory" value={gb(snap.mem_used_mb)} sub={`of ${gb(snap.mem_total_mb, 0)}`}>
          <div className="meters">
            <Meter
              label="Physical"
              value={`${pct(memFrac * 100, 0)} · ${gb(snap.mem_used_mb)} / ${gb(snap.mem_total_mb, 0)}`}
              fraction={memFrac}
              level={mem}
              caption={
                mem === 'nominal'
                  ? 'Kernel memory pressure normal'
                  : `Kernel memory pressure ${snap.mem_pressure} — the machine is compressing or swapping`
              }
            />
            <Meter
              label="Swap"
              value={gb(snap.swap_used_mb)}
              fraction={swapFrac}
              caption="Relative to physical memory. macOS grows the swap file on demand, so its own size says little."
            />
            <dl className="kv">
              <dt>Thermal state</dt>
              <dd>{snap.thermal_state}</dd>
              <dt>CPU · GPU temp</dt>
              <dd>
                {celsius(snap.temp_cpu_c)} · {celsius(snap.temp_gpu_c)}
              </dd>
              {snap.fans && snap.fans.length > 0 && (
                <>
                  <dt>Fans</dt>
                  <dd>{snap.fans.map(f => `${f.name} ${f.rpm} rpm`).join(' · ')}</dd>
                </>
              )}
            </dl>
          </div>
        </ChartCard>
      </section>

      {top && <ProcessTable top={top} />}
    </main>
  )
}
