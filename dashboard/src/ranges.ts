export const RANGES = [
  { sec: 120, label: '2 min' },
  { sec: 300, label: '5 min' },
  { sec: 900, label: '15 min' },
] as const

export type RangeSec = (typeof RANGES)[number]['sec']
