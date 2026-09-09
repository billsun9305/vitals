import type { Level } from '../format'

interface Props {
  label: string
  value: string
  /** 0–1. */
  fraction: number
  level?: Level
  caption?: string
}

/**
 * A single ratio against a limit. The fill carries severity and the track is
 * a lighter step of the same ramp, so the state reads across the whole bar.
 */
export function Meter({ label, value, fraction, level = 'nominal', caption }: Props) {
  const w = Math.min(Math.max(fraction, 0), 1) * 100
  return (
    <div className={`meter meter--${level}`}>
      <div className="meter-head">
        <span>{label}</span>
        <strong>{value}</strong>
      </div>
      <div
        className="track"
        role="meter"
        aria-label={label}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.round(w)}
      >
        <div className="fill" style={{ width: `${w}%` }} />
      </div>
      {caption && <p className="meter-caption">{caption}</p>}
    </div>
  )
}
