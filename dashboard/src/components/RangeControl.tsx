import { RANGES, type RangeSec } from '../ranges'

/** The one filter row. Everything below it is scoped to the chosen window. */
export function RangeControl({ value, onChange }: { value: RangeSec; onChange: (s: RangeSec) => void }) {
  return (
    <div className="range" role="group" aria-label="History window">
      {RANGES.map(r => (
        <button key={r.sec} aria-pressed={value === r.sec} onClick={() => onChange(r.sec)}>
          {value === r.sec && (
            <svg viewBox="0 0 12 12" aria-hidden="true">
              <path d="M2 6.5l2.5 2.5L10 3.5" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
            </svg>
          )}
          {r.label}
        </button>
      ))}
    </div>
  )
}
