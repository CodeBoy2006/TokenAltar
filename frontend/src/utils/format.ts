export function fmt(value: number | null | undefined, digits = 2) {
  return Number(value || 0).toLocaleString(undefined, { maximumFractionDigits: digits })
}

export function surgeStateLabel(value: string | undefined) {
  const labels: Record<string, string> = {
    idle: 'Idle',
    normal: 'Normal',
    peak: 'Peak',
    no_capacity: 'No capacity',
  }
  return labels[value || 'idle'] || 'Idle'
}

export function compactDate(value: string | null | undefined) {
  if (!value) return 'now'
  const normalized = value.includes('T') ? value : value.replace(' ', 'T')
  const date = new Date(normalized.endsWith('Z') ? normalized : `${normalized}Z`)
  if (Number.isNaN(date.getTime())) return value
  return date.toLocaleString(undefined, {
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  })
}

export function splitCsv(value: string) {
  return value.split(',').map((item) => item.trim()).filter(Boolean)
}

export function optionalNumber(value: number | string | null | undefined) {
  if (value === null || value === undefined || value === '') return null
  const parsed = Number(value)
  return Number.isFinite(parsed) ? parsed : null
}
