// Mirrors `crates/core/src/schema.rs` field-for-field. Units live in the key
// names (`_mb`, `_pct`, `_w`, `_c`) on the Rust side, so the same convention
// is kept here rather than re-deriving or renaming anything.

export type PressureLevel = 'normal' | 'warning' | 'critical' | 'unknown'
export type ThermalState = 'nominal' | 'fair' | 'serious' | 'critical' | 'unknown'
export type Severity = 'nominal' | 'warning' | 'critical'
export type CoreKind = 'E' | 'P'

export interface Host {
  chip: string
  model: string
  ecpu_cores: number
  pcpu_cores: number
  gpu_cores: number
  /** Logical CPUs as the scheduler sees them; the denominator for load average. */
  ncpu: number
}

export interface Core {
  /** Index in the concatenation of E cores then P cores. */
  id: number
  kind: CoreKind
  die_id: number
  core_id: number
  pct: number
  freq_mhz: number
}

export interface Fan {
  name: string
  rpm: number
  /** Omitted by the server (never `null`) when unknown. */
  max_rpm?: number
}

export interface Snapshot {
  schema_version: number
  sampled_at: string
  sample_ms: number
  host: Host
  cpu_pct: number
  cpu_scaled_pct: number
  ecpu_pct: number
  ecpu_freq_mhz: number
  pcpu_pct: number
  pcpu_freq_mhz: number
  cores: Core[]
  gpu_pct: number
  gpu_freq_mhz: number
  mem_total_mb: number
  mem_used_mb: number
  swap_total_mb: number
  swap_used_mb: number
  mem_pressure: PressureLevel
  power_total_w: number
  power_cpu_w: number
  power_gpu_w: number
  power_ane_w: number
  power_ram_w: number
  power_sys_w: number
  temp_cpu_c: number
  temp_gpu_c: number
  /** Omitted by the server (never `null`/empty key) when there are no fans. */
  fans?: Fan[]
  load_avg: [number, number, number]
  uptime_s: number
  thermal_state: ThermalState
}

export interface ProcRow {
  pid: number
  name: string
  app?: string
  /** Per-process CPU percent; exceeds 100 on multi-core work, like `ps`. */
  cpu_pct: number
  mem_mb: number
  threads: number
}

export interface TopReport {
  schema_version: number
  sampled_at: string
  sample_ms: number
  by_cpu: ProcRow[]
  by_mem: ProcRow[]
}

export interface Reason {
  code: string
  severity: Severity
  detail: string
}

export interface Suspect {
  pid: number
  name: string
  app?: string
  why: string
}

export interface Verdict {
  schema_version: number
  sampled_at: string
  state: Severity
  /** False when no usable prior observation existed yet. */
  history: boolean
  reasons: Reason[]
  suspects: Suspect[]
  summary: string
}
