import type { Verdict as VerdictData } from '../types'

const TITLE = { nominal: 'Healthy', warning: 'Under pressure', critical: 'Critical' } as const

function Icon({ state }: { state: VerdictData['state'] }) {
  const cls = `verdict-icon verdict-icon--${state}`
  if (state === 'nominal') {
    return (
      <svg className={cls} viewBox="0 0 20 20" aria-hidden="true">
        <circle cx="10" cy="10" r="9" fill="currentColor" />
        <path d="M6 10.5l2.5 2.5L14 7.5" fill="none" stroke="#fff" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
      </svg>
    )
  }
  if (state === 'warning') {
    return (
      <svg className={cls} viewBox="0 0 20 20" aria-hidden="true">
        <path d="M10 2l9 16H1z" fill="currentColor" />
        <path d="M10 8v4.5M10 15.2v.3" stroke="#fff" strokeWidth="2" strokeLinecap="round" />
      </svg>
    )
  }
  return (
    <svg className={cls} viewBox="0 0 20 20" aria-hidden="true">
      <path d="M6.5 1h7L19 6.5v7L13.5 19h-7L1 13.5v-7z" fill="currentColor" />
      <path d="M10 6v5M10 14v.5" stroke="#fff" strokeWidth="2" strokeLinecap="round" />
    </svg>
  )
}

/**
 * The headline: the machine's health in one line, with the reasons and the
 * processes the verdict blames. Status is icon + label, never colour alone.
 */
export function Verdict({ v }: { v: VerdictData }) {
  return (
    <section className={`verdict verdict--${v.state}`} aria-live="polite">
      <Icon state={v.state} />
      <h2 className="verdict-title">{TITLE[v.state]}</h2>
      <p className="verdict-summary">{v.summary}</p>
      {(v.reasons.length > 0 || v.suspects.length > 0) && (
        <div className="verdict-detail">
          {v.reasons.length > 0 && (
            <ul>
              {v.reasons.map(r => (
                <li key={r.code}>
                  <span className={`dot dot--${r.severity === 'nominal' ? 'good' : r.severity}`} aria-hidden="true" />
                  {r.detail}
                </li>
              ))}
            </ul>
          )}
          {v.suspects.length > 0 && (
            <div className="chips">
              {v.suspects.map(s => (
                <span className="chip" key={s.pid}>
                  <strong>{s.app ?? s.name}</strong> · {s.why}
                </span>
              ))}
            </div>
          )}
        </div>
      )}
    </section>
  )
}
