export function tokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`
  return String(n)
}

export function cost(usd: number): string {
  return `$${usd.toFixed(2)}`
}

/// Callers must pass `null` when the source timestamp was absent — a real session record
/// can omit `statusUpdatedAt` entirely (observed on a just-started session), and formatting
/// `now - 0` as a duration renders ~56 years.
export function elapsed(ms: number | null): string {
  if (ms === null) return '—'
  const s = Math.max(0, Math.floor(ms / 1000))
  if (s < 60) return `${s}s`
  if (s < 3600) return `${Math.floor(s / 60)}m`
  return `${Math.floor(s / 3600)}h`
}
