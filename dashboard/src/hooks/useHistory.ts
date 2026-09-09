import { useCallback, useState } from 'react'

export interface Sample {
  t: number
  cpu: number
  gpu: number
  memUsedMb: number
  powerW: number
}

/**
 * Fifteen minutes at the 1 Hz snapshot cadence. The range control only ever
 * narrows what is drawn; the buffer is what makes widening it instant.
 */
export const MAX_HISTORY = 900

export function useHistory(): { samples: Sample[]; push: (s: Sample) => void } {
  const [samples, setSamples] = useState<Sample[]>([])
  const push = useCallback((s: Sample) => {
    setSamples(h => (h.length >= MAX_HISTORY ? [...h.slice(1), s] : [...h, s]))
  }, [])
  return { samples, push }
}
