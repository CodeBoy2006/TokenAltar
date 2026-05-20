import type { TabId, TabItem } from '../api/types'

export type TabMeta = {
  eyebrow: string
  title: string
  description: string
}

export const adminOnlyTabs = new Set<TabId>(['users', 'affinity', 'settings'])

export const tabDetails: Record<TabId, TabMeta> = {
  dashboard: {
    eyebrow: 'Capacity Atrium',
    title: 'Gateway dashboard',
    description: 'Live token supply, surge pressure, and the service routes exposed to clients.',
  },
  users: {
    eyebrow: 'Steward Registry',
    title: 'User management',
    description: 'Create accounts, adjust roles and balances, reset credentials, and suspend access.',
  },
  keys: {
    eyebrow: 'Credential Gallery',
    title: 'API key registry',
    description: 'Issue controlled client keys and watch spend against each local allowance.',
  },
  channels: {
    eyebrow: 'Provider Colonnade',
    title: 'Channel inventory',
    description: 'Shape upstream pools with model coverage, quota windows, and fire-sale economics.',
  },
  health: {
    eyebrow: 'Pulse Arcade',
    title: 'Channel health',
    description: 'Passive request-derived health windows, TTFT, and provider status across upstream capacity.',
  },
  prices: {
    eyebrow: 'Tariff Tablet',
    title: 'Pricing rules',
    description: 'Map model patterns to input, output, and cache-token settlement rates.',
  },
  affinity: {
    eyebrow: 'Binding Frieze',
    title: 'Affinity rules',
    description: 'Keep tenants, sessions, and cache-sensitive traffic on stable routing lanes.',
  },
  economy: {
    eyebrow: 'Social Treasury',
    title: 'Point economy',
    description: 'Move points between users, create phrase packets, and control ranking visibility.',
  },
  leaderboards: {
    eyebrow: 'Monthly Honors',
    title: 'Leaderboards',
    description: 'Provider token contribution and consumer point burn, grouped by current month.',
  },
  ledger: {
    eyebrow: 'Settlement Archive',
    title: 'Ledger entries',
    description: 'Trace usage, tokenizer decisions, and point formulas behind every settlement.',
  },
  guide: {
    eyebrow: 'Relief Guide',
    title: 'Project guide',
    description: 'A visual map of the TokenAltar flow from users and keys to routing, economy, and health.',
  },
  settings: {
    eyebrow: 'Admin Chamber',
    title: 'Console settings',
    description: 'Local invite controls for a gated TokenAltar circle.',
  },
}

export const tabBackgrounds: Partial<Record<TabId, string>> = {
  dashboard: '/backgrounds/console-dashboard-overview.png',
  keys: '/backgrounds/console-api-keys-vault.png',
  health: '/backgrounds/console-channel-health.png',
  prices: '/backgrounds/console-pricing-rules.png',
  economy: '/backgrounds/console-point-economy.png',
  leaderboards: '/backgrounds/console-leaderboards-honors.png',
  settings: '/backgrounds/console-settings-chamber.png',
}

export function visibleTabs(isAdmin: boolean): TabItem[] {
  const items: TabItem[] = [
    ['dashboard', 'Dashboard'],
    ['keys', 'API Keys'],
    ['health', 'Health'],
    ['channels', 'Channels'],
    ['prices', 'Pricing'],
    ['economy', 'Economy'],
    ['leaderboards', 'Leaderboards'],
    ['ledger', 'Ledger'],
    ['guide', 'Guide'],
  ]
  if (isAdmin) {
    items.splice(1, 0, ['users', 'Users'])
    items.splice(5, 0, ['affinity', 'Affinity'])
    items.push(['settings', 'Settings'])
  }
  return items
}
