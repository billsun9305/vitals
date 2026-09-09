interface SparklineProps {
  values: number[]
  width?: number
  height?: number
  max?: number
  label: string
}

/** A minimal 0-100 (or 0-`max`) percent sparkline, no charting library. */
export function Sparkline({ values, width = 600, height = 64, max = 100, label }: SparklineProps) {
  const points = values
    .map((v, i) => {
      const x = (i / Math.max(values.length - 1, 1)) * width
      const clamped = Math.min(Math.max(v, 0), max)
      const y = height - (clamped / max) * height
      return `${x.toFixed(1)},${y.toFixed(1)}`
    })
    .join(' ')

  return (
    <svg
      width={width}
      height={height}
      viewBox={`0 0 ${width} ${height}`}
      className="sparkline"
      role="img"
      aria-label={label}
      preserveAspectRatio="none"
    >
      {values.length > 1 && (
        <polyline points={points} fill="none" stroke="currentColor" strokeWidth={1.5} />
      )}
    </svg>
  )
}
