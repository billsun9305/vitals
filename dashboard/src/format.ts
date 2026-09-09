// Formatting is centralised so a unit is never spelled two ways on one page.

import type { Core } from './types'

export const pct = (v: number, digits = 1): string => `${v.toFixed(digits)}%`

export const gb = (mb: number, digits = 1): string => `${(mb / 1024).toFixed(digits)} GB`

export const watts = (w: number): string => `${w.toFixed(2)} W`

export const celsius = (c: number): string => `${Math.round(c)}°C`

export const mhz = (v: number): string => `${v.toLocaleString()} MHz`

export function uptime(totalSeconds: number): string {
  const d = Math.floor(totalSeconds / 86400)
  const h = Math.floor((totalSeconds % 86400) / 3600)
  const m = Math.floor((totalSeconds % 3600) / 60)
  if (d > 0) return `${d}d ${h}h`
  if (h > 0) return `${h}h ${m}m`
  return `${m}m`
}

/** `12:04:05` in the viewer's locale, 24-hour so columns stay aligned. */
export function clock(t: number): string {
  return new Date(t).toLocaleTimeString(undefined, { hour12: false })
}

/** An axis label for "this many seconds before now". */
export function ago(seconds: number): string {
  if (seconds <= 0) return 'now'
  if (seconds < 60) return `−${Math.round(seconds)}s`
  const m = seconds / 60
  return `−${Number.isInteger(m) ? m : m.toFixed(1)}m`
}

export type Level = 'nominal' | 'warning' | 'critical'

/** The dashboard's own threshold rule, shared with the menu bar dropdown. */
export function levelFor(pctValue: number): Level {
  if (pctValue >= 90) return 'critical'
  if (pctValue >= 70) return 'warning'
  return 'nominal'
}

/**
 * `E0 E1 P0 … P7`. The API's `core_id` is the cluster encoding the kernel
 * reports (0, 10, 20 … 130), which is a name, not a count; people read cores
 * by position.
 */
export function coreLabels(cores: Core[]): string[] {
  const seen: Record<string, number> = {}
  return cores.map(c => `${c.kind}${(seen[c.kind] = (seen[c.kind] ?? -1) + 1)}`)
}
