import type { Channel, ChannelHealthSummary, ChannelHealthWindow } from '../api/types'
import { fmt } from './format'

export type HealthTotals = {
  samples: number
  available: number
  empty: number
  degraded: number
  down: number
  avgTtftMs: number | null
}

export function healthWindows(channel: Pick<Channel, 'health_windows'>): ChannelHealthWindow[] {
  return Array.isArray(channel.health_windows) ? channel.health_windows : []
}

export function healthSummary(channel: Pick<Channel, 'health_windows'>): ChannelHealthSummary {
  const windows = healthWindows(channel)
  const current = windows[windows.length - 1]
  if (current && current.sample_count > 0) {
    const labelByStatus: Record<ChannelHealthWindow['status'], string> = {
      available: 'Available',
      empty: 'Empty',
      degraded: 'Degraded',
      down: 'Down',
      unknown: 'Gray',
    }
    const toneByStatus: Record<ChannelHealthWindow['status'], ChannelHealthSummary['tone']> = {
      available: 'olive',
      empty: 'gold',
      degraded: 'lapis',
      down: 'wine',
      unknown: 'gray',
    }
    const ttft = current.avg_ttft_ms === null ? 'TTFT n/a' : `TTFT ${fmt(current.avg_ttft_ms, 0)}ms`
    const sample = `${current.sample_count} sample${current.sample_count === 1 ? '' : 's'}`
    return {
      label: labelByStatus[current.status],
      detail: `${sample} · ${ttft}`,
      tone: toneByStatus[current.status],
    }
  }
  return {
    label: 'Gray',
    detail: 'No records in current window',
    tone: 'gray',
  }
}

export function healthBarClass(window: ChannelHealthWindow) {
  return {
    gray: window.sample_count === 0 || window.status === 'unknown',
    olive: window.status === 'available' && window.success_count > 0 && window.empty_count === 0 && window.degraded_count === 0 && window.down_count === 0,
    gold: window.status === 'empty' || (window.status === 'available' && window.empty_count > 0),
    lapis: window.status === 'degraded' || (window.status === 'available' && window.degraded_count > 0),
    wine: window.status === 'down' || (window.status === 'available' && window.down_count > 0),
  }
}

export function healthWindowTitle(window: ChannelHealthWindow) {
  const ttft = window.avg_ttft_ms === null ? 'TTFT n/a' : `TTFT ${fmt(window.avg_ttft_ms, 0)}ms`
  return [
    `${window.window_start_at} -> ${window.window_end_at}`,
    `status: ${window.status}`,
    `samples: ${window.sample_count}`,
    `available: ${window.success_count}`,
    `empty: ${window.empty_count}`,
    `degraded: ${window.degraded_count}`,
    `down: ${window.down_count}`,
    ttft,
  ].join(' · ')
}

export function healthTotals(channel: Pick<Channel, 'health_windows'>): HealthTotals {
  const windows = healthWindows(channel)
  let ttftNumerator = 0
  let ttftDenominator = 0
  const totals = windows.reduce((acc, window) => {
    acc.samples += window.sample_count
    acc.available += window.success_count
    acc.empty += window.empty_count
    acc.degraded += window.degraded_count
    acc.down += window.down_count
    if (window.avg_ttft_ms !== null && window.success_count > 0) {
      ttftNumerator += window.avg_ttft_ms * window.success_count
      ttftDenominator += window.success_count
    }
    return acc
  }, {
    samples: 0,
    available: 0,
    empty: 0,
    degraded: 0,
    down: 0,
  })
  return {
    ...totals,
    avgTtftMs: ttftDenominator > 0 ? ttftNumerator / ttftDenominator : null,
  }
}

export function healthCurrentWindow(channel: Pick<Channel, 'health_windows'>) {
  const windows = healthWindows(channel)
  return windows[windows.length - 1] || null
}

export function providerTone(provider: string) {
  const normalized = String(provider || '').toLowerCase()
  return {
    openai: normalized === 'openai',
    anthropic: normalized === 'anthropic',
    gemini: normalized === 'gemini',
  }
}

export function primaryWindow(channel: Pick<Channel, 'limits'>) {
  return channel.limits?.windows?.[0] || null
}

export function quotaSummary(channel: Pick<Channel, 'limits'>) {
  return (channel.limits?.windows || [])
    .map((window) => `${window.name}: ${fmt(window.limit_tokens - window.used_tokens, 0)}`)
    .join(' / ') || '-'
}

export function statusClass(record: { enabled?: boolean; status?: string }) {
  return {
    off: !record.enabled || record.status === 'manual_disabled',
    warn: record.status === 'cooling',
    danger: record.status === 'deleted',
  }
}
