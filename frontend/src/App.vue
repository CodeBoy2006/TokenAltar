<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, ref } from 'vue'
import { apiRequest } from './api/client'
import type {
  AffinityRule,
  ApiKeyCreateResponse,
  ApiKeyRecord,
  AuthResponse,
  Channel,
  ChannelQuotaWindow,
  ChannelTestResult,
  Dashboard,
  EditableChannelWindow,
  LedgerEntry,
  LeaderboardPayload,
  LeaderboardRow,
  ManagedUser,
  ModelPrice,
  RankedLeaderboardRow,
  RedPacketRecord,
  RuntimeSettings,
  SettingRecord,
  SettingsPayload,
  TabId,
  TransferRecord,
  User,
} from './api/types'
import HealthStrip from './components/HealthStrip.vue'
import ProviderBadge from './components/ProviderBadge.vue'
import { compactDate, fmt, optionalNumber, splitCsv, surgeStateLabel } from './utils/format'
import {
  healthCurrentWindow,
  healthSummary,
  healthTotals,
  healthWindows,
  primaryWindow,
  quotaSummary,
  statusClass,
} from './utils/health'
import { useConsoleEvents } from './composables/useConsoleEvents'
import { adminOnlyTabs, tabBackgrounds, tabDetails, visibleTabs } from './config/tabs'

const token = ref(localStorage.getItem('tokenaltar_token') || '')
const user = ref<User | null>(null)
const error = ref('')
const activeTab = ref<TabId>('dashboard')
const authMode = ref<'login' | 'register'>('login')
const apiKeys = ref<ApiKeyRecord[]>([])
const users = ref<ManagedUser[]>([])
const channels = ref<Channel[]>([])
const prices = ref<ModelPrice[]>([])
const rules = ref<AffinityRule[]>([])
const ledger = ref<LedgerEntry[]>([])
const transfers = ref<TransferRecord[]>([])
const redPackets = ref<RedPacketRecord[]>([])
const leaderboards = ref<LeaderboardPayload>({ providers: [], consumers: [] })
const leaderboardPeriod = ref<'day' | 'month'>('month')
const settings = ref<SettingRecord[]>([])
const runtimeSettings = ref<RuntimeSettings>({})
const dashboard = ref<Dashboard | null>(null)
const newApiKey = ref('')
const claimResult = ref('')
const routeChannels = ref<Channel[]>([])
const selectedApiKeyIds = ref<number[]>([])
const selectedChannelIds = ref<number[]>([])
const editingUserId = ref<number | null>(null)
const editingApiKeyId = ref<number | null>(null)
const editingChannelId = ref<number | null>(null)
const channelTestResults = ref<Record<number, string>>({})
const apiKeyChannelModalOpen = ref(false)
const apiKeyChannelFilter = ref('')
const apiKeyChannelAssignedFilter = ref('')
const draggedApiKeyChannelId = ref<number | null>(null)
const apiKeyFilter = ref('')
const channelFilter = ref('')
const healthFilter = ref('')
const userFilter = ref('')
const selectedPricePreset = ref('')

type PricePreset = {
  key: string
  label: string
  pattern: string
  input: number
  output: number
  cache: number
}

type GuideMechanism = {
  index: string
  badge: string
  title: string
  summary: string
  points: string[]
}

const pricePresets: PricePreset[] = [
  { key: 'gpt-5.5', label: 'GPT-5.5', pattern: '^gpt-5\\.5$', input: 5, output: 30, cache: 0.5 },
  { key: 'gpt-5.4', label: 'GPT-5.4', pattern: '^gpt-5\\.4$', input: 2.5, output: 15, cache: 0.25 },
  { key: 'gpt-5.3-codex', label: 'GPT-5.3 Codex', pattern: '^gpt-5\\.3-codex$', input: 2.5, output: 15, cache: 0.25 },
  { key: 'gpt-5.2', label: 'GPT-5.2', pattern: '^gpt-5\\.2$', input: 2.5, output: 15, cache: 0.25 },
  { key: 'gpt-5.2-codex', label: 'GPT-5.2 Codex', pattern: '^gpt-5\\.2-codex$', input: 2.5, output: 15, cache: 0.25 },
  { key: 'claude-opus-4.7', label: 'Claude Opus 4.7', pattern: '^claude-opus-4[.-]7(-.+)?$', input: 5, output: 25, cache: 0.5 },
  { key: 'claude-opus-4.6', label: 'Claude Opus 4.6', pattern: '^claude-opus-4[.-]6(-.+)?$', input: 5, output: 25, cache: 0.5 },
  { key: 'claude-opus-4.5', label: 'Claude Opus 4.5', pattern: '^claude-opus-4[.-]5(-.+)?$', input: 5, output: 25, cache: 0.5 },
  { key: 'claude-opus-4.1', label: 'Claude Opus 4.1', pattern: '^claude-opus-4[.-]1(-.+)?$', input: 15, output: 75, cache: 1.5 },
  { key: 'claude-opus-4', label: 'Claude Opus 4', pattern: '^claude-opus-4(-.+)?$', input: 15, output: 75, cache: 1.5 },
  { key: 'claude-sonnet-4.6', label: 'Claude Sonnet 4.6', pattern: '^claude-sonnet-4[.-]6(-.+)?$', input: 3, output: 15, cache: 0.3 },
  { key: 'claude-sonnet-4.5', label: 'Claude Sonnet 4.5', pattern: '^claude-sonnet-4[.-]5(-.+)?$', input: 3, output: 15, cache: 0.3 },
  { key: 'claude-sonnet-4', label: 'Claude Sonnet 4', pattern: '^claude-sonnet-4(-.+)?$', input: 3, output: 15, cache: 0.3 },
  { key: 'claude-haiku-4.5', label: 'Claude Haiku 4.5', pattern: '^claude-haiku-4[.-]5(-.+)?$', input: 1, output: 5, cache: 0.1 },
]

const loginForm = reactive({ email: '', password: '' })
const registerForm = reactive({ email: '', password: '', display_name: '', invite_code: '' })
const userForm = reactive({
  email: '',
  password: '',
  role: 'user',
  display_name: '',
  points_balance: 0,
  enabled: true,
})
const apiKeyForm = reactive({
  name: 'local-dev',
  spend_limit_points: null as number | null,
  enabled: true,
  expires_at: '',
  allowed_models: '',
  allowed_channel_ids: [] as number[],
})
const channelForm = reactive({
  name: '',
  provider: '',
  base_url: '',
  api_key_secret: '',
  models: '',
  enabled: true,
  windows: [] as EditableChannelWindow[],
  fire_sale_days_before: null as number | null,
  fire_sale_remaining_pct: null as number | null,
  fire_sale_discount: null as number | null,
  provider_share: null as number | null,
})
const priceForm = reactive({
  channel_id: null as number | null,
  model_pattern: 'default',
  input_price_per_1m: null as number | null,
  output_price_per_1m: null as number | null,
  cache_price_per_1m: null as number | null,
})
const ruleForm = reactive({
  name: 'tenant-session',
  enabled: true,
  model_regex: '.*',
  request_path: '/v1/chat/completions',
  user_agent_regex: '',
  key_source_type: 'request_header',
  key_source_path: 'x-tenant-id',
  group_name: 'default',
  ttl_seconds: 3600,
  skip_retry_on_failure: false,
  switch_on_success: true,
  include_model_name: true,
})
const transferForm = reactive({ to_user_id: 0, points: 10, memo: '@TokenAltar PayTo:' })
const redPacketForm = reactive({ phrase: 'RustIsBest', total_points: 30, total_parts: 3, mode: 'even' })
const claimForm = reactive({ phrase: 'RustIsBest' })
const settingsForm = reactive<Record<string, string>>({
  invite_required: 'false',
  invite_code_default: 'TOKENALTAR',
})
const settingsSchema = [
  { key: 'invite_required', label: 'Invite Required', type: 'boolean' },
  { key: 'invite_code_default', label: 'Default Invite Code', type: 'text' },
  { key: 'initial_admin_points', label: 'Initial Admin Points', type: 'number' },
  { key: 'initial_user_points', label: 'Initial User Points', type: 'number' },
  { key: 'settlement_round_digits', label: 'Settlement Round Digits', type: 'number' },
  { key: 'fallback_input_price_per_unit', label: 'Fallback Input Price / 1M', type: 'number' },
  { key: 'fallback_output_price_per_unit', label: 'Fallback Output Price / 1M', type: 'number' },
  { key: 'fallback_cache_price_per_unit', label: 'Fallback Cache Price / 1M', type: 'number' },
  { key: 'surge_low_threshold', label: 'Surge Low Threshold', type: 'number' },
  { key: 'surge_high_threshold', label: 'Surge High Threshold', type: 'number' },
  { key: 'surge_idle_multiplier', label: 'Surge Idle Multiplier', type: 'number' },
  { key: 'surge_normal_multiplier', label: 'Surge Normal Multiplier', type: 'number' },
  { key: 'surge_peak_multiplier', label: 'Surge Peak Multiplier', type: 'number' },
  { key: 'routing_max_attempts', label: 'Routing Max Attempts', type: 'number' },
  { key: 'routing_retry_cooldown_seconds', label: 'Retry Cooldown Seconds', type: 'number' },
  { key: 'routing_fire_sale_weight_multiplier', label: 'Fire Sale Route Weight', type: 'number' },
  { key: 'ledger_queue_capacity', label: 'Ledger Queue Capacity', type: 'number' },
  { key: 'affinity_cache_capacity', label: 'Affinity Cache Capacity', type: 'number' },
  { key: 'default_api_key_spend_limit_points', label: 'Default API Key Spend Limit', type: 'number' },
  { key: 'default_channel_name', label: 'Default Channel Name', type: 'text' },
  { key: 'default_channel_provider', label: 'Default Channel Provider', type: 'text' },
  { key: 'default_channel_base_url', label: 'Default Channel Base URL', type: 'text' },
  { key: 'default_channel_models', label: 'Default Channel Models', type: 'text' },
  { key: 'default_channel_windows_json', label: 'Default Channel Windows JSON', type: 'textarea' },
  { key: 'default_channel_fire_sale_days_before', label: 'Default Fire Sale Days', type: 'number' },
  { key: 'default_channel_fire_sale_remaining_pct', label: 'Default Fire Sale Remaining', type: 'number' },
  { key: 'default_channel_fire_sale_discount', label: 'Default Fire Sale Discount', type: 'number' },
  { key: 'default_channel_provider_share', label: 'Default Provider Share', type: 'number' },
]

const isAdmin = computed(() => user.value?.role === 'admin')
const tabs = computed(() => visibleTabs(isAdmin.value))
const activeTabMeta = computed(() => {
  const meta = tabDetails[activeTab.value]
  if (!isAdmin.value && activeTab.value === 'channels') {
    return {
      ...meta,
      title: 'My channels',
      description: 'Manage your own upstream capacity, quotas, pricing inputs, and health checks.',
    }
  }
  if (!isAdmin.value && activeTab.value === 'health') {
    return {
      ...meta,
      title: 'My channel health',
      description: 'Passive request-derived health windows, TTFT, and provider status for your upstream pools.',
    }
  }
  if (!isAdmin.value && activeTab.value === 'prices') {
    return {
      ...meta,
      title: 'Channel pricing',
      description: 'Set rates for your channels; admin-managed defaults remain visible as fallback rows.',
    }
  }
  return meta
})
const consoleBackgroundStyle = computed(() => ({
  '--console-page-bg': `url(${tabBackgrounds[activeTab.value] || '/tokenaltar-background.png'})`,
}))
const editingApiKey = computed(() => apiKeys.value.find((item) => item.id === editingApiKeyId.value))
const editingUser = computed(() => users.value.find((item) => item.id === editingUserId.value))
const editingChannel = computed(() => channels.value.find((item) => item.id === editingChannelId.value))
const filteredUsers = computed(() => {
  const needle = userFilter.value.trim().toLowerCase()
  if (!needle) return users.value
  return users.value.filter((record) => {
    const haystack = [
      record.id,
      record.email,
      record.display_name,
      record.role,
      record.enabled ? 'enabled' : 'disabled',
    ].join(' ').toLowerCase()
    return haystack.includes(needle)
  })
})
const filteredApiKeys = computed(() => {
  const needle = apiKeyFilter.value.trim().toLowerCase()
  if (!needle) return apiKeys.value
  return apiKeys.value.filter((key) => {
    const haystack = [
      key.name,
      key.key_prefix,
      key.enabled ? 'enabled' : 'disabled',
      ...(key.allowed_models || []),
    ].join(' ').toLowerCase()
    return haystack.includes(needle)
  })
})
const apiKeyChannelOptions = computed(() => routeChannels.value.length > 0 ? routeChannels.value : channels.value)
const selectedApiKeyChannels = computed(() => {
  const selected = new Set(apiKeyForm.allowed_channel_ids)
  return apiKeyChannelOptions.value.filter((channel) => selected.has(channel.id))
})
const availableApiKeyChannels = computed(() => {
  const selected = new Set(apiKeyForm.allowed_channel_ids)
  return apiKeyChannelOptions.value.filter((channel) => !selected.has(channel.id))
})
const filteredAvailableApiKeyChannels = computed(() => filterApiKeyChannels(availableApiKeyChannels.value, apiKeyChannelFilter.value))
const filteredSelectedApiKeyChannels = computed(() => filterApiKeyChannels(selectedApiKeyChannels.value, apiKeyChannelAssignedFilter.value))
const filteredChannels = computed(() => {
  const needle = channelFilter.value.trim().toLowerCase()
  if (!needle) return channels.value
  return channels.value.filter((channel) => {
    const haystack = [
      channel.name,
      channel.provider,
      channel.status,
      channel.base_url,
      ownerLabel(channel),
      ...(channel.models || []),
    ].join(' ').toLowerCase()
    return haystack.includes(needle)
  })
})
const filteredHealthChannels = computed(() => {
  const needle = healthFilter.value.trim().toLowerCase()
  if (!needle) return channels.value
  return channels.value.filter((channel) => {
    const summary = healthSummary(channel)
    const totals = healthTotals(channel)
    const haystack = [
      channel.name,
      channel.provider,
      channel.status,
      channel.enabled ? 'enabled' : 'disabled',
      channel.base_url,
      ownerLabel(channel),
      summary.label,
      summary.detail,
      totals.avgTtftMs === null ? 'ttft n/a' : `ttft ${fmt(totals.avgTtftMs, 0)}ms`,
      ...(channel.models || []),
    ].join(' ').toLowerCase()
    return haystack.includes(needle)
  })
})
const allFilteredApiKeysSelected = computed(() =>
  filteredApiKeys.value.length > 0 && filteredApiKeys.value.every((key) => selectedApiKeyIds.value.includes(key.id)),
)
const allFilteredChannelsSelected = computed(() =>
  filteredChannels.value.length > 0 && filteredChannels.value.every((channel) => selectedChannelIds.value.includes(channel.id)),
)
const dashboardMetrics = computed(() => [
  { label: 'Surge', value: surgeStateLabel(dashboard.value?.surge_state), detail: `${dashboard.value?.surge_multiplier || 1}x multiplier` },
  { label: 'Available tokens', value: fmt(dashboard.value?.available_tokens, 0), detail: 'ready for routing' },
  { label: 'Enabled channels', value: `${dashboard.value?.enabled_channels || 0} / ${dashboard.value?.channels || 0}`, detail: 'online capacity' },
  { label: 'Today spend', value: fmt(dashboard.value?.spent_points_today, 4), detail: 'points settled' },
])
const healthMetrics = computed(() => {
  const totals = channels.value.reduce((acc, channel) => {
    const windows = healthWindows(channel)
    const channelTotals = healthTotals(channel)
    acc.samples += channelTotals.samples
    acc.available += channelTotals.available
    acc.empty += channelTotals.empty
    acc.degraded += channelTotals.degraded
    acc.down += channelTotals.down
    acc.unknown += windows.filter((window) => window.sample_count === 0).length
    if (channel.enabled && channel.status !== 'deleted') acc.enabled += 1
    if (channelTotals.avgTtftMs !== null) {
      acc.ttftNumerator += channelTotals.avgTtftMs * channelTotals.available
      acc.ttftDenominator += channelTotals.available
    }
    return acc
  }, {
    samples: 0,
    available: 0,
    empty: 0,
    degraded: 0,
    down: 0,
    unknown: 0,
    enabled: 0,
    ttftNumerator: 0,
    ttftDenominator: 0,
  })
  const avgTtft = totals.ttftDenominator > 0 ? totals.ttftNumerator / totals.ttftDenominator : null
  return [
    { label: 'Channels', value: `${channels.value.length}`, detail: `${totals.enabled} enabled` },
    { label: 'Samples', value: fmt(totals.samples, 0), detail: `${totals.available} available / ${totals.empty} empty` },
    { label: 'Down windows', value: fmt(totals.down, 0), detail: `${totals.degraded} degraded / ${totals.unknown} gray` },
    { label: 'Avg TTFT', value: avgTtft === null ? 'n/a' : `${fmt(avgTtft, 0)}ms`, detail: 'successful non-empty only' },
  ]
})
const providerRows = computed(() => rankedRows(leaderboards.value.providers, 'tokens', 0))
const consumerRows = computed(() => rankedRows(leaderboards.value.consumers, 'points', 4))
const leaderboardSummary = computed(() => {
  const providerTotal = totalScore(leaderboards.value.providers)
  const consumerTotal = totalScore(leaderboards.value.consumers)
  const providerTop = providerRows.value[0]
  const consumerTop = consumerRows.value[0]

  return [
    {
      label: 'Provider volume',
      value: fmt(providerTotal, 0),
      detail: providerTop ? `${providerTop.name} leads` : 'no provider rows',
    },
    {
      label: 'Consumer burn',
      value: fmt(consumerTotal, 4),
      detail: consumerTop ? `${consumerTop.name} leads` : 'no consumer rows',
    },
    {
      label: 'Ranked stewards',
      value: `${leaderboards.value.providers.length + leaderboards.value.consumers.length}`,
      detail: `${leaderboards.value.timezone || 'server-local'} window`,
    },
  ]
})
const priceSaveDisabled = computed(() => !isAdmin.value && !priceForm.channel_id)
const guideOnboardingMode = computed(() =>
  runtimeSettings.value.invite_required ? 'invite-gated membership' : 'open registration',
)
const guideCapacityMode = computed(() =>
  (runtimeSettings.value.default_channel_windows || []).length > 0 ? 'windowed capacity policy' : 'operator-defined capacity policy',
)
const guidePricingMode = computed(() =>
  runtimeSettings.value.pricing_unit_tokens === 1000000 ? 'per-million token tariff' : 'configured token tariff',
)
const guideCanonIntro = computed(() =>
  `TokenAltar is governed as an ${guideOnboardingMode.value} with ${guideCapacityMode.value}, ${guidePricingMode.value}, and passive health records. The wording follows the active console policy without exposing raw keys, channel inventory, quotas, prices, or multipliers.`,
)
const guideMechanisms = computed<GuideMechanism[]>(() => [
  {
    index: 'I',
    badge: guideOnboardingMode.value,
    title: 'Access Charter',
    summary: 'Members enter through console accounts and call models through TokenAltar-issued client keys. The current membership policy controls who may join the circle.',
    points: [
      'Client keys can be fenced by model family and permitted routing lanes.',
      'Disabled accounts lose console sessions, key access, and active channel routing.',
    ],
  },
  {
    index: 'II',
    badge: guideCapacityMode.value,
    title: 'Capacity Covenant',
    summary: 'Providers contribute upstream channels, each declaring model coverage and capacity windows. Every active window is treated as a hard routing constraint.',
    points: [
      'The primary window guides availability, route weight, and end-of-window fire-sale behavior.',
      'Provider secrets remain sealed inside channel records while owner-scoped views keep capacity manageable.',
    ],
  },
  {
    index: 'III',
    badge: 'affinity-aware routing',
    title: 'Routing Rite',
    summary: 'Each request passes through key scope, model coverage, quota state, health, cooldown, and affinity checks before a channel is chosen.',
    points: [
      'Cache-sensitive or tenant-bound traffic can stay on stable lanes for a configured lifetime.',
      'Retryable failures and empty semantic replies may switch lanes before meaningful content reaches the client.',
    ],
  },
  {
    index: 'IV',
    badge: guidePricingMode.value,
    title: 'Settlement Canon',
    summary: 'Usage settles after upstream usage is known. Channel tariffs override global model patterns, and global patterns override the runtime fallback policy.',
    points: [
      'Input, output, and cache tokens are accounted as separate settlement surfaces.',
      'Surge and fire-sale policies adjust the final point flow without changing the client API.',
    ],
  },
  {
    index: 'V',
    badge: 'passive health watch',
    title: 'Health Chronicle',
    summary: 'Channel health is learned from real traffic instead of scheduled probes. Windows distinguish available, empty, degraded, and down outcomes.',
    points: [
      'TTFT is averaged from successful non-empty samples only.',
      'The console refreshes affected health panels through live topic invalidations.',
    ],
  },
  {
    index: 'VI',
    badge: 'social treasury',
    title: 'Point Economy',
    summary: 'Points connect model consumption, provider rewards, transfers, phrase packets, and leaderboards into one visible economy.',
    points: [
      'Provider boards rank token supply; consumer boards rank settled point burn.',
      'Ranking visibility follows each steward profile while ledger history remains accountable.',
    ],
  },
])

async function api<T = unknown>(path: string, options: RequestInit = {}) {
  error.value = ''
  return apiRequest<T>(path, token.value, options)
}

async function login() {
  try {
    const data = await api<AuthResponse>('/auth/login', { method: 'POST', body: JSON.stringify(loginForm) })
    acceptAuth(data)
  } catch (err) {
    error.value = String(err)
  }
}

async function register() {
  try {
    const data = await api<AuthResponse>('/auth/register', { method: 'POST', body: JSON.stringify(registerForm) })
    acceptAuth(data)
  } catch (err) {
    error.value = String(err)
  }
}

async function acceptAuth(data: AuthResponse) {
  token.value = data.token
  user.value = data.user
  localStorage.setItem('tokenaltar_token', data.token)
  await refreshAll()
  startConsoleEventStream()
}

function logout() {
  stopConsoleEventStream()
  token.value = ''
  user.value = null
  activeTab.value = 'dashboard'
  localStorage.removeItem('tokenaltar_token')
}

async function refreshAll() {
  if (!token.value) return
  try {
    user.value = await api<User>('/me')
    ensureAllowedTab()
    if (!isAdmin.value) {
      users.value = []
      rules.value = []
      settings.value = []
    }
    await loadRuntimeSettings()
    await Promise.all([
      loadDashboard(),
      isAdmin.value ? loadUsers() : Promise.resolve(),
      loadRouteChannels(),
      loadChannels(),
      loadPrices(),
      isAdmin.value ? loadRules() : Promise.resolve(),
      loadLedger(),
      loadTransfers(),
      loadRedPackets(),
      loadLeaderboards(),
      isAdmin.value ? loadSettings() : Promise.resolve(),
    ])
  } catch (err) {
    error.value = String(err)
    logout()
  }
}

const { start: startConsoleEventStream, stop: stopConsoleEventStream } = useConsoleEvents({
  getToken: () => token.value,
  onUnauthorized: () => logout(),
  onTopics: refreshConsoleTopics,
  onError: (err) => {
    error.value = String(err)
    logout()
  },
})

async function refreshConsoleTopics(topics: Set<string>) {
  if (!token.value) return
  if (topics.has('sync')) {
    await refreshAll()
    return
  }
  const tasks: Promise<unknown>[] = []
  if (topics.has('me')) tasks.push(refreshMe().then(ensureAllowedTab))
  if (topics.has('runtimeSettings')) tasks.push(loadRuntimeSettings())
  if (topics.has('dashboard')) tasks.push(loadDashboard())
  if (topics.has('users') && isAdmin.value) tasks.push(loadUsers())
  if (topics.has('apiKeys')) tasks.push(loadRouteChannels())
  if (topics.has('channels')) tasks.push(Promise.all([loadChannels(), loadRouteChannels()]))
  if (topics.has('prices')) tasks.push(loadPrices())
  if (topics.has('affinityRules') && isAdmin.value) tasks.push(loadRules())
  if (topics.has('ledger')) tasks.push(loadLedger())
  if (topics.has('transfers')) tasks.push(loadTransfers())
  if (topics.has('redPackets')) tasks.push(loadRedPackets())
  if (topics.has('leaderboards')) tasks.push(loadLeaderboards())
  if (topics.has('settings') && isAdmin.value) tasks.push(loadSettings())
  await Promise.all(tasks)
}

async function loadDashboard() { dashboard.value = await api<Dashboard>('/dashboard') }
async function loadUsers() { users.value = await api<ManagedUser[]>('/users') }
async function loadApiKeys() { apiKeys.value = await api<ApiKeyRecord[]>('/api-keys') }
async function loadRouteChannels() {
  routeChannels.value = await api<Channel[]>('/route-channels')
  pruneApiKeyChannelSelection()
  applyDefaultApiKeyChannels()
  await loadApiKeys()
}
async function loadChannels() {
  channels.value = await api<Channel[]>('/channels')
  pruneApiKeyChannelSelection()
  if (!isAdmin.value && channels.value.length === 0) {
    priceForm.channel_id = null
  } else if (!isAdmin.value && !channels.value.some((channel) => channel.id === priceForm.channel_id)) {
    priceForm.channel_id = channels.value[0].id
  }
}
async function loadPrices() { prices.value = await api<ModelPrice[]>('/prices') }
async function loadRules() { rules.value = await api<AffinityRule[]>('/affinity-rules') }
async function loadLedger() { ledger.value = await api<LedgerEntry[]>('/ledger') }
async function loadTransfers() { transfers.value = await api<TransferRecord[]>('/transfers') }
async function loadRedPackets() { redPackets.value = await api<RedPacketRecord[]>('/red-packets') }
async function loadLeaderboards() { leaderboards.value = await api<LeaderboardPayload>(`/leaderboards?period=${leaderboardPeriod.value}`) }

async function loadRuntimeSettings() {
  runtimeSettings.value = await api<RuntimeSettings>('/runtime-settings')
  applyRuntimeDefaults()
}

async function loadSettings() {
  const payload = await api<SettingsPayload>('/settings')
  settings.value = payload.settings || []
  runtimeSettings.value = payload.runtime || runtimeSettings.value
  for (const setting of settings.value) {
    settingsForm[setting.key] = setting.value
  }
  applyRuntimeDefaults()
}

async function createManagedUser() {
  const created = await api<ManagedUser>('/users', {
    method: 'POST',
    body: JSON.stringify({
      email: userForm.email,
      password: userForm.password,
      role: userForm.role,
      display_name: userForm.display_name || null,
      points_balance: optionalNumber(userForm.points_balance),
      enabled: userForm.enabled,
    }),
  })
  editingUserId.value = created.id
  userForm.password = ''
  await Promise.all([loadUsers(), loadDashboard()])
}

async function saveManagedUser() {
  if (!editingUserId.value) return
  const updated = await api<ManagedUser>(`/users/${editingUserId.value}`, {
    method: 'PATCH',
    body: JSON.stringify({
      email: userForm.email,
      role: userForm.role,
      display_name: userForm.display_name,
      points_balance: Number(userForm.points_balance),
      enabled: userForm.enabled,
    }),
  })
  if (updated.id === user.value?.id) {
    await refreshMe()
    ensureAllowedTab()
  }
  await Promise.all([loadUsers(), loadDashboard(), loadChannels()])
}

async function toggleManagedUser(record: ManagedUser) {
  const updated = await api<ManagedUser>(`/users/${record.id}/enabled`, {
    method: 'POST',
    body: JSON.stringify({ enabled: !record.enabled }),
  })
  if (updated.id === user.value?.id) {
    await refreshMe()
    ensureAllowedTab()
  }
  await Promise.all([loadUsers(), loadDashboard(), loadChannels()])
}

async function resetManagedUserPassword() {
  if (!editingUserId.value || !userForm.password) return
  await api(`/users/${editingUserId.value}/password`, {
    method: 'POST',
    body: JSON.stringify({ password: userForm.password }),
  })
  userForm.password = ''
}

function selectManagedUser(record: ManagedUser) {
  editingUserId.value = record.id
  userForm.email = record.email
  userForm.password = ''
  userForm.role = record.role
  userForm.display_name = record.display_name
  userForm.points_balance = Number(record.points_balance)
  userForm.enabled = record.enabled
}

function resetManagedUserForm() {
  editingUserId.value = null
  userForm.email = ''
  userForm.password = ''
  userForm.role = 'user'
  userForm.display_name = ''
  userForm.points_balance = runtimeSettings.value.initial_user_points ?? 0
  userForm.enabled = true
}

async function createApiKey() {
  const data = await api<ApiKeyCreateResponse>('/api-keys', {
    method: 'POST',
    body: JSON.stringify(apiKeyPayload()),
  })
  newApiKey.value = data.token
  editingApiKeyId.value = data.record.id
  await loadApiKeys()
  selectApiKey(data.record)
}

async function toggleApiKey(record: ApiKeyRecord) {
  await api(`/api-keys/${record.id}/enabled`, {
    method: 'POST',
    body: JSON.stringify({ enabled: !record.enabled }),
  })
  await loadApiKeys()
}

async function saveApiKey() {
  if (!editingApiKeyId.value) return
  await api(`/api-keys/${editingApiKeyId.value}`, {
    method: 'PATCH',
    body: JSON.stringify(apiKeyPayload()),
  })
  await loadApiKeys()
}

async function rotateApiKey(record: ApiKeyRecord) {
  const data = await api<ApiKeyCreateResponse>(`/api-keys/${record.id}/rotate`, { method: 'POST' })
  newApiKey.value = data.token
  editingApiKeyId.value = record.id
  await loadApiKeys()
}

async function deleteApiKey(record: ApiKeyRecord) {
  await api(`/api-keys/${record.id}`, { method: 'DELETE' })
  selectedApiKeyIds.value = selectedApiKeyIds.value.filter((id) => id !== record.id)
  if (editingApiKeyId.value === record.id) resetApiKeyForm()
  await loadApiKeys()
}

async function deleteSelectedApiKeys() {
  if (selectedApiKeyIds.value.length === 0) return
  await api('/api-keys/batch-delete', {
    method: 'POST',
    body: JSON.stringify({ ids: selectedApiKeyIds.value }),
  })
  resetApiKeyForm()
  selectedApiKeyIds.value = []
  await loadApiKeys()
}

function toggleFilteredApiKeys() {
  if (allFilteredApiKeysSelected.value) {
    const visible = new Set(filteredApiKeys.value.map((key) => key.id))
    selectedApiKeyIds.value = selectedApiKeyIds.value.filter((id) => !visible.has(id))
    return
  }
  selectedApiKeyIds.value = Array.from(new Set([
    ...selectedApiKeyIds.value,
    ...filteredApiKeys.value.map((key) => key.id),
  ]))
}

function filterApiKeyChannels(source: Channel[], query: string) {
  const needle = query.trim().toLowerCase()
  if (!needle) return source
  return source.filter((channel) => channelSearchText(channel).includes(needle))
}

function channelSearchText(channel: Channel) {
  const totals = healthTotals(channel)
  return [
    channel.name,
    channel.provider,
    channel.status,
    channel.enabled ? 'enabled' : 'disabled',
    channel.base_url,
    ownerLabel(channel),
    healthSummary(channel).label,
    quotaSummary(channel),
    totals.avgTtftMs === null ? 'ttft n/a' : `ttft ${fmt(totals.avgTtftMs, 0)}ms`,
    ...(channel.models || []),
  ].join(' ').toLowerCase()
}

function normalizeChannelIds(ids: unknown[]) {
  const visible = new Set(apiKeyChannelOptions.value.map((channel) => channel.id))
  const normalized: number[] = []
  for (const id of ids) {
    const numeric = Number(id)
    if (Number.isInteger(numeric) && visible.has(numeric) && !normalized.includes(numeric)) {
      normalized.push(numeric)
    }
  }
  return normalized
}

function pruneApiKeyChannelSelection() {
  apiKeyForm.allowed_channel_ids = normalizeChannelIds(apiKeyForm.allowed_channel_ids)
}

function openApiKeyChannelModal() {
  pruneApiKeyChannelSelection()
  apiKeyChannelModalOpen.value = true
}

function closeApiKeyChannelModal() {
  draggedApiKeyChannelId.value = null
  apiKeyChannelModalOpen.value = false
}

function addApiKeyChannel(channelId: number) {
  if (!apiKeyForm.allowed_channel_ids.includes(channelId)) {
    apiKeyForm.allowed_channel_ids.push(channelId)
  }
}

function removeApiKeyChannel(channelId: number) {
  apiKeyForm.allowed_channel_ids = apiKeyForm.allowed_channel_ids.filter((id) => id !== channelId)
}

function selectAllApiKeyChannels() {
  apiKeyForm.allowed_channel_ids = apiKeyChannelOptions.value.map((channel) => channel.id)
}

function clearApiKeyChannels() {
  apiKeyForm.allowed_channel_ids = []
}

function selectFilteredApiKeyChannels() {
  for (const channel of filteredAvailableApiKeyChannels.value) {
    addApiKeyChannel(channel.id)
  }
}

function clearFilteredApiKeyChannels() {
  const removeIds = new Set(filteredSelectedApiKeyChannels.value.map((channel) => channel.id))
  apiKeyForm.allowed_channel_ids = apiKeyForm.allowed_channel_ids.filter((id) => !removeIds.has(id))
}

function startApiKeyChannelDrag(event: DragEvent, channelId: number) {
  draggedApiKeyChannelId.value = channelId
  event.dataTransfer?.setData('text/plain', String(channelId))
  if (event.dataTransfer) {
    event.dataTransfer.effectAllowed = 'move'
  }
}

function dropApiKeyChannel(event: DragEvent, target: 'available' | 'selected') {
  const rawId = event.dataTransfer?.getData('text/plain')
  const channelId = Number(rawId || draggedApiKeyChannelId.value)
  if (!Number.isInteger(channelId)) return
  if (target === 'selected') {
    addApiKeyChannel(channelId)
  } else {
    removeApiKeyChannel(channelId)
  }
  draggedApiKeyChannelId.value = null
}

function channelSelectionLabel(ids: unknown[]) {
  const selected = ids.filter((id) => Number.isInteger(Number(id)))
  if (apiKeyChannelOptions.value.length === 0) return 'No channels available'
  if (selected.length === 0) return 'No channels selected'
  if (selected.length === apiKeyChannelOptions.value.length) return 'All route channels'
  return `${selected.length} / ${apiKeyChannelOptions.value.length} channels`
}

function apiKeyChannelNames(ids: unknown[]) {
  const selected = new Set(ids.map((id) => Number(id)).filter(Number.isInteger))
  const names = apiKeyChannelOptions.value
    .filter((channel) => selected.has(channel.id))
    .slice(0, 3)
    .map((channel) => channel.name)
  if (names.length === 0) return 'none'
  const extra = selected.size - names.length
  return extra > 0 ? `${names.join(', ')} +${extra}` : names.join(', ')
}

function channelCardTitle(channel: Channel) {
  return [
    channel.name,
    ownerLabel(channel),
    channel.provider,
    channel.models?.join(', ') || '*',
    healthSummary(channel).detail,
  ].join(' / ')
}

function selectApiKey(record: ApiKeyRecord) {
  editingApiKeyId.value = record.id
  apiKeyForm.name = record.name
  apiKeyForm.spend_limit_points = record.spend_limit_points
  apiKeyForm.enabled = record.enabled
  apiKeyForm.expires_at = record.expires_at || ''
  apiKeyForm.allowed_models = (record.allowed_models || []).join(', ')
  apiKeyForm.allowed_channel_ids = normalizeChannelIds(record.allowed_channel_ids || [])
  apiKeyChannelFilter.value = ''
  apiKeyChannelAssignedFilter.value = ''
}

function resetApiKeyForm() {
  editingApiKeyId.value = null
  apiKeyForm.name = 'local-dev'
  apiKeyForm.spend_limit_points = runtimeSettings.value.default_api_key_spend_limit_points ?? null
  apiKeyForm.enabled = true
  apiKeyForm.expires_at = ''
  apiKeyForm.allowed_models = ''
  apiKeyForm.allowed_channel_ids = apiKeyChannelOptions.value.map((channel) => channel.id)
  apiKeyChannelFilter.value = ''
  apiKeyChannelAssignedFilter.value = ''
  apiKeyChannelModalOpen.value = false
}

function applyDefaultApiKeyChannels() {
  if (!editingApiKeyId.value && !apiKeyChannelModalOpen.value && apiKeyForm.allowed_channel_ids.length === 0) {
    apiKeyForm.allowed_channel_ids = apiKeyChannelOptions.value.map((channel) => channel.id)
  }
}

async function createChannel() {
  await api('/channels', {
    method: 'POST',
    body: JSON.stringify({
      ...channelPayload(),
      api_key_secret: channelForm.api_key_secret,
    }),
  })
  channelForm.api_key_secret = ''
  await Promise.all([loadChannels(), loadDashboard()])
  if (!isAdmin.value && !priceForm.channel_id && channels.value.length > 0) {
    priceForm.channel_id = channels.value[0].id
  }
}

async function saveChannel() {
  if (!editingChannelId.value) return
  await api(`/channels/${editingChannelId.value}`, {
    method: 'PATCH',
    body: JSON.stringify(channelPayload()),
  })
  channelForm.api_key_secret = ''
  await Promise.all([loadChannels(), loadDashboard()])
}

async function toggleChannel(channel: Channel) {
  await api(`/channels/${channel.id}/enabled`, {
    method: 'POST',
    body: JSON.stringify({ enabled: !channel.enabled }),
  })
  await Promise.all([loadChannels(), loadDashboard()])
}

async function deleteChannel(channel: Channel) {
  await api(`/channels/${channel.id}`, { method: 'DELETE' })
  selectedChannelIds.value = selectedChannelIds.value.filter((id) => id !== channel.id)
  if (editingChannelId.value === channel.id) resetChannelForm()
  await Promise.all([loadChannels(), loadDashboard()])
}

async function copyChannel(channel: Channel) {
  const copied = await api<Channel>(`/channels/${channel.id}/copy`, {
    method: 'POST',
    body: JSON.stringify({ suffix: ' copy', reset_usage: true }),
  })
  await Promise.all([loadChannels(), loadDashboard()])
  selectChannel(copied)
}

async function testChannel(channel: Channel) {
  channelTestResults.value[channel.id] = 'testing...'
  const result = await api<ChannelTestResult>(`/channels/${channel.id}/test`, { method: 'POST' })
  channelTestResults.value[channel.id] = `${result.ok ? 'OK' : 'Failed'} / ${result.latency_ms}ms / ${result.message}`
  await loadChannels()
}

async function setSelectedChannels(enabled: boolean) {
  if (selectedChannelIds.value.length === 0) return
  await api('/channels/batch-enabled', {
    method: 'POST',
    body: JSON.stringify({ ids: selectedChannelIds.value, enabled }),
  })
  await Promise.all([loadChannels(), loadDashboard()])
}

function selectChannel(channel: Channel) {
  editingChannelId.value = channel.id
  channelForm.name = channel.name
  channelForm.provider = channel.provider
  channelForm.base_url = channel.base_url
  channelForm.api_key_secret = ''
  channelForm.models = (channel.models || []).join(', ')
  channelForm.enabled = channel.enabled
  channelForm.windows = cloneWindows(channel.limits.windows || [])
  channelForm.fire_sale_days_before = channel.limits.fire_sale_days_before
  channelForm.fire_sale_remaining_pct = channel.limits.fire_sale_remaining_pct
  channelForm.fire_sale_discount = channel.limits.fire_sale_discount
  channelForm.provider_share = channel.limits.provider_share
}

function resetChannelForm() {
  editingChannelId.value = null
  channelForm.name = runtimeSettings.value.default_channel_name || ''
  channelForm.provider = runtimeSettings.value.default_channel_provider || ''
  channelForm.base_url = runtimeSettings.value.default_channel_base_url || ''
  channelForm.api_key_secret = ''
  channelForm.models = runtimeSettings.value.default_channel_models || ''
  channelForm.enabled = true
  channelForm.windows = defaultWindows()
  channelForm.fire_sale_days_before = runtimeSettings.value.default_channel_fire_sale_days_before ?? null
  channelForm.fire_sale_remaining_pct = runtimeSettings.value.default_channel_fire_sale_remaining_pct ?? null
  channelForm.fire_sale_discount = runtimeSettings.value.default_channel_fire_sale_discount ?? null
  channelForm.provider_share = runtimeSettings.value.default_channel_provider_share ?? null
}

function defaultWindows() {
  const anchor = defaultAnchor()
  return (runtimeSettings.value.default_channel_windows || []).map((window) => ({
    ...window,
    anchor_at: anchor,
  }))
}

function defaultAnchor() {
  return new Date().toISOString().slice(0, 19)
}

function cloneWindows(windows: Partial<ChannelQuotaWindow>[]) {
  return windows.map((window) => ({
    name: window.name || 'Window',
    limit_tokens: Number(window.limit_tokens || 0),
    period_unit: window.period_unit || 'day',
    period_count: Number(window.period_count || 1),
    anchor_at: window.anchor_at || defaultAnchor(),
    timezone: window.timezone || 'UTC',
  }))
}

function addQuotaWindow() {
  const template = runtimeSettings.value.default_channel_windows?.[0]
  channelForm.windows.push({
    name: template?.name || 'Window',
    limit_tokens: Number(template?.limit_tokens || 1),
    period_unit: template?.period_unit || 'day',
    period_count: Number(template?.period_count || 1),
    anchor_at: defaultAnchor(),
    timezone: template?.timezone || 'UTC',
  })
}

function removeQuotaWindow(index: number) {
  if (channelForm.windows.length <= 1) return
  channelForm.windows.splice(index, 1)
}

function toggleFilteredChannels() {
  if (allFilteredChannelsSelected.value) {
    const visible = new Set(filteredChannels.value.map((channel) => channel.id))
    selectedChannelIds.value = selectedChannelIds.value.filter((id) => !visible.has(id))
    return
  }
  selectedChannelIds.value = Array.from(new Set([
    ...selectedChannelIds.value,
    ...filteredChannels.value.map((channel) => channel.id),
  ]))
}

async function savePrice() {
  if (priceSaveDisabled.value) {
    error.value = 'Add a channel before setting channel-specific prices.'
    return
  }
  await api('/prices', { method: 'POST', body: JSON.stringify(priceForm) })
  await loadPrices()
}

function applyPricePreset() {
  const preset = pricePresets.find((item) => item.key === selectedPricePreset.value)
  if (!preset) return
  priceForm.model_pattern = preset.pattern
  priceForm.input_price_per_1m = preset.input
  priceForm.output_price_per_1m = preset.output
  priceForm.cache_price_per_1m = preset.cache
}

function priceScope(price: ModelPrice) {
  if (!price.channel_id) return isAdmin.value ? 'Global default' : 'Default fallback'
  const channel = channels.value.find((item) => item.id === price.channel_id)
  return channel ? channelOptionLabel(channel) : `Channel #${price.channel_id}`
}

function isGlobalPrice(price: ModelPrice) {
  return !price.channel_id
}

function ownerLabel(record: { owner_user_id?: number | null; owner_display_name?: string | null }) {
  if (record?.owner_display_name) return record.owner_display_name
  return record?.owner_user_id ? `User #${record.owner_user_id}` : '-'
}

function channelOptionLabel(channel: Channel) {
  return isAdmin.value ? `${channel.name} (${ownerLabel(channel)})` : channel.name
}

function priceOwnerLabel(price: ModelPrice) {
  if (!price.channel_id) return 'System'
  const channel = channels.value.find((item) => item.id === price.channel_id)
  return channel ? ownerLabel(channel) : '-'
}

function ensureAllowedTab() {
  if (user.value && adminOnlyTabs.has(activeTab.value) && !isAdmin.value) {
    activeTab.value = 'dashboard'
  }
}

async function createRule() {
  await api('/affinity-rules', {
    method: 'POST',
    body: JSON.stringify({
      ...ruleForm,
      user_agent_regex: ruleForm.user_agent_regex || null,
      model_regex: ruleForm.model_regex || null,
    }),
  })
  await loadRules()
}

async function transferPoints() {
  await api('/transfers', { method: 'POST', body: JSON.stringify(transferForm) })
  await Promise.all([loadTransfers(), refreshMe()])
}

async function createRedPacket() {
  await api('/red-packets', { method: 'POST', body: JSON.stringify(redPacketForm) })
  await Promise.all([loadRedPackets(), refreshMe()])
}

async function claimRedPacket() {
  const data = await api<{ points: number }>('/red-packets/claim', { method: 'POST', body: JSON.stringify(claimForm) })
  claimResult.value = `Claimed ${data.points.toFixed(4)} points`
  await Promise.all([loadRedPackets(), refreshMe()])
}

async function toggleAnonymous() {
  const updated = await api<User>('/profile/anonymous-leaderboard', {
    method: 'POST',
    body: JSON.stringify({ enabled: !user.value?.anonymous_leaderboard }),
  })
  user.value = updated
  await loadLeaderboards()
}

async function setLeaderboardPeriod(period: 'day' | 'month') {
  leaderboardPeriod.value = period
  await loadLeaderboards()
}

async function saveSettings() {
  await api('/settings', {
    method: 'POST',
    body: JSON.stringify(settingsSchema.map((item) => ({
      key: item.key,
      value: settingsForm[item.key],
    }))),
  })
  await loadSettings()
}

function applyRuntimeDefaults() {
  if (!priceForm.channel_id && channels.value.length > 0) {
    priceForm.channel_id = channels.value[0].id
  }
  applyDefaultApiKeyChannels()
  if (!editingApiKeyId.value && apiKeyForm.spend_limit_points === null) {
    apiKeyForm.spend_limit_points = runtimeSettings.value.default_api_key_spend_limit_points ?? null
  }
  channelForm.name = channelForm.name || runtimeSettings.value.default_channel_name || ''
  channelForm.provider = channelForm.provider || runtimeSettings.value.default_channel_provider || ''
  channelForm.base_url = channelForm.base_url || runtimeSettings.value.default_channel_base_url || ''
  channelForm.models = channelForm.models || runtimeSettings.value.default_channel_models || ''
  if (!channelForm.windows.length) {
    channelForm.windows = defaultWindows()
  }
  if (channelForm.fire_sale_days_before === null) {
    channelForm.fire_sale_days_before = runtimeSettings.value.default_channel_fire_sale_days_before ?? null
  }
  if (channelForm.fire_sale_remaining_pct === null) {
    channelForm.fire_sale_remaining_pct = runtimeSettings.value.default_channel_fire_sale_remaining_pct ?? null
  }
  if (channelForm.fire_sale_discount === null) {
    channelForm.fire_sale_discount = runtimeSettings.value.default_channel_fire_sale_discount ?? null
  }
  if (channelForm.provider_share === null) {
    channelForm.provider_share = runtimeSettings.value.default_channel_provider_share ?? null
  }
  if (priceForm.input_price_per_1m === null) {
    priceForm.input_price_per_1m = runtimeSettings.value.fallback_input_price_per_unit ?? null
  }
  if (priceForm.output_price_per_1m === null) {
    priceForm.output_price_per_1m = runtimeSettings.value.fallback_output_price_per_unit ?? null
  }
  if (priceForm.cache_price_per_1m === null) {
    priceForm.cache_price_per_1m = runtimeSettings.value.fallback_cache_price_per_unit ?? null
  }
  if (!editingUserId.value && !userForm.email && userForm.points_balance === 0) {
    userForm.points_balance = runtimeSettings.value.initial_user_points ?? 0
  }
}

async function refreshMe() {
  user.value = await api<User>('/me')
}

function transferDirection(item: TransferRecord) {
  return item.to_user_id === user.value?.id ? 'In' : 'Out'
}

function signedTransferPoints(item: TransferRecord) {
  const sign = item.to_user_id === user.value?.id ? '+' : '-'
  return `${sign}${fmt(item.points, 4)}`
}

function packetClaimedPct(packet: RedPacketRecord) {
  const total = Number(packet.total_parts || 0)
  if (total <= 0) return 0
  return Math.min(100, Math.max(0, Math.round((Number(packet.claimed_parts || 0) / total) * 100)))
}

function formatTtft(value: number | null | undefined) {
  return value === null || value === undefined ? 'n/a' : `${fmt(value, 0)}ms`
}

function totalScore(rows: LeaderboardRow[] = []) {
  return rows.reduce((sum, row) => sum + Number(row.score || 0), 0)
}

function rankedRows(rows: LeaderboardRow[] = [], unit: 'tokens' | 'points', digits: number): RankedLeaderboardRow[] {
  const peak = Math.max(...rows.map((row) => Number(row.score || 0)), 0)
  const tones: RankedLeaderboardRow['tone'][] = ['gold', 'lapis', 'olive']

  return rows.map((row, index) => {
    const score = Number(row.score || 0)
    return {
      ...row,
      key: `${row.user_id ?? 'anonymous'}:${row.name}:${index}`,
      rank: index + 1,
      scoreText: `${fmt(score, digits)} ${unit}`,
      share: peak > 0 ? Math.max(4, Math.round((score / peak) * 100)) : 0,
      tone: tones[index] || 'plain',
    }
  })
}

function apiKeyPayload() {
  return {
    name: apiKeyForm.name,
    enabled: apiKeyForm.enabled,
    spend_limit_points: optionalNumber(apiKeyForm.spend_limit_points),
    expires_at: apiKeyForm.expires_at || null,
    allowed_models: splitCsv(apiKeyForm.allowed_models),
    allowed_channel_ids: normalizeChannelIds(apiKeyForm.allowed_channel_ids),
  }
}

function channelPayload() {
  return {
    name: channelForm.name,
    provider: channelForm.provider,
    base_url: channelForm.base_url,
    enabled: channelForm.enabled,
    windows: channelForm.windows.map((window) => ({
      name: window.name,
      limit_tokens: Number(window.limit_tokens),
      period_unit: window.period_unit,
      period_count: Number(window.period_count),
      anchor_at: window.anchor_at,
      timezone: window.timezone,
    })),
    fire_sale_days_before: Number(channelForm.fire_sale_days_before),
    fire_sale_remaining_pct: Number(channelForm.fire_sale_remaining_pct),
    fire_sale_discount: Number(channelForm.fire_sale_discount),
    provider_share: Number(channelForm.provider_share),
    api_key_secret: channelForm.api_key_secret || null,
    models: splitCsv(channelForm.models),
  }
}

onMounted(async () => {
  await refreshAll()
  if (token.value) startConsoleEventStream()
})
onBeforeUnmount(stopConsoleEventStream)
</script>

<template>
  <main class="shell" :class="{ 'auth-shell': !user, 'modal-open': apiKeyChannelModalOpen }" :style="consoleBackgroundStyle">
    <aside v-if="user" class="sidebar">
      <div class="brand">
        <div class="mark"><span>TA</span></div>
        <div>
          <h1>TokenAltar</h1>
          <p>Token-native LLM gateway</p>
        </div>
      </div>
      <nav v-if="user" class="tabs">
        <button v-for="[id, label] in tabs" :key="id" :class="{ active: activeTab === id }" @click="activeTab = id">
          <span class="tab-glyph" aria-hidden="true"></span>
          {{ label }}
        </button>
      </nav>
      <div v-if="user" class="account">
        <span class="account-kicker">Current steward</span>
        <strong>{{ user.display_name }}</strong>
        <span>#{{ user.id }} / {{ user.role }}</span>
        <span>{{ fmt(user.points_balance, 4) }} points</span>
        <button class="ghost light" @click="logout">Sign out</button>
      </div>
    </aside>

    <section class="content">
      <div v-if="error" class="error">{{ error }}</div>

      <section v-if="!user" class="auth-panel">
        <div class="auth-hero">
          <div class="hero-atmosphere" aria-hidden="true"></div>
          <div class="hero-symbols" aria-hidden="true">
            <div class="antikythera-dial">
              <span class="dial-ring ring-major"></span>
              <span class="dial-ring ring-minor"></span>
              <span class="gear gear-large"></span>
              <span class="gear gear-small"></span>
              <span class="dial-hand"></span>
            </div>
          </div>
          <div class="hero-copy">
            <div class="hero-kicker">
              <span>Private token exchange</span>
              <i aria-hidden="true"></i>
              <span>LLM capacity console</span>
            </div>
            <h2>Token<wbr />Altar</h2>
          </div>
          <div class="auth-card">
            <div class="auth-card-header">
              <h3>{{ authMode === 'login' ? 'Sign in' : 'Create account' }}</h3>
              <div class="segmented auth-mode" aria-label="Authentication mode">
                <button type="button" :class="{ active: authMode === 'login' }" @click="authMode = 'login'">Login</button>
                <button type="button" :class="{ active: authMode === 'register' }" @click="authMode = 'register'">Register</button>
              </div>
            </div>
            <template v-if="authMode === 'login'">
              <form class="auth-form" @submit.prevent="login">
                <label>Email <input v-model="loginForm.email" autocomplete="username" /></label>
                <label>Password <input v-model="loginForm.password" type="password" autocomplete="current-password" /></label>
                <button type="submit">Enter console</button>
              </form>
            </template>
            <template v-else>
              <form class="auth-form" @submit.prevent="register">
                <label>Email <input v-model="registerForm.email" autocomplete="username" /></label>
                <label>Name <input v-model="registerForm.display_name" autocomplete="name" /></label>
                <label>Password <input v-model="registerForm.password" type="password" autocomplete="new-password" /></label>
                <label>Invite code <input v-model="registerForm.invite_code" autocomplete="off" /></label>
                <button type="submit">Create account</button>
              </form>
            </template>
          </div>
        </div>
      </section>

      <template v-else>
        <header class="page-header">
          <div>
            <span>{{ activeTabMeta.eyebrow }}</span>
            <h2>{{ activeTabMeta.title }}</h2>
            <p>{{ activeTabMeta.description }}</p>
          </div>
          <div class="header-stat">
            <span>Balance</span>
            <strong>{{ fmt(user.points_balance, 4) }}</strong>
            <small>points</small>
          </div>
        </header>

        <section v-if="activeTab === 'dashboard'">
          <div class="toolbar">
            <div><h3>Operational waterline</h3><p>Gateway health and token economy at a glance.</p></div>
            <button class="ghost" @click="refreshAll">Refresh</button>
          </div>
          <div class="metric-grid">
            <article v-for="metric in dashboardMetrics" :key="metric.label">
              <span>{{ metric.label }}</span>
              <strong>{{ metric.value }}</strong>
              <em>{{ metric.detail }}</em>
            </article>
          </div>
          <div class="endpoint-strip">
            <code>POST /v1/chat/completions</code>
            <code>POST /v1/responses</code>
            <code>POST /v1/messages</code>
          </div>
        </section>

        <section v-if="activeTab === 'health'" class="health-page">
          <div class="health-hero-panel">
            <div>
              <span class="section-kicker">Passive monitor</span>
              <h3>All channel windows</h3>
              <p>Each upstream channel is grouped by provider with request-derived availability, empty replies, down samples, and TTFT windows.</p>
            </div>
            <div class="health-actions">
              <input v-model="healthFilter" class="search-input" placeholder="Filter channels, providers, owners, models" />
              <button class="ghost" @click="loadChannels">Refresh</button>
            </div>
          </div>

          <div class="metric-grid health-metrics">
            <article v-for="metric in healthMetrics" :key="metric.label">
              <span>{{ metric.label }}</span>
              <strong>{{ metric.value }}</strong>
              <em>{{ metric.detail }}</em>
            </article>
          </div>

          <div class="health-board">
            <article v-for="channel in filteredHealthChannels" :key="channel.id" class="channel-health-card" :class="{ disabled: !channel.enabled }">
              <div class="channel-health-head">
                <div class="channel-identity">
                  <div class="channel-title-line">
                    <h3>{{ channel.name }}</h3>
                    <ProviderBadge :provider="channel.provider" />
                  </div>
                  <div class="channel-subtitle">
                    <span>{{ ownerLabel(channel) }}</span>
                    <span>{{ channel.models.join(', ') || '*' }}</span>
                  </div>
                </div>
                <span class="status" :class="statusClass(channel)">{{ channel.enabled ? channel.status : 'disabled' }}</span>
              </div>

              <div class="channel-health-current">
                <div class="health-summary card-summary">
                  <strong :class="`tone-${healthSummary(channel).tone}`">{{ healthSummary(channel).label }}</strong>
                  <span>{{ healthSummary(channel).detail }}</span>
                </div>
                <div class="current-window-time">
                  {{ healthCurrentWindow(channel)?.window_start_at || '-' }} -> {{ healthCurrentWindow(channel)?.window_end_at || '-' }}
                </div>
              </div>

              <HealthStrip
                :windows="healthWindows(channel)"
                :channel-name="channel.name"
                :key-prefix="`${channel.id}:health-page`"
                variant="large"
              />

              <div class="health-stat-row">
                <span><strong>{{ healthTotals(channel).samples }}</strong> samples</span>
                <span><strong>{{ healthTotals(channel).available }}</strong> available</span>
                <span><strong>{{ healthTotals(channel).empty }}</strong> empty</span>
                <span><strong>{{ healthTotals(channel).down }}</strong> down</span>
                <span><strong>{{ formatTtft(healthTotals(channel).avgTtftMs) }}</strong> avg TTFT</span>
              </div>
            </article>
            <div v-if="filteredHealthChannels.length === 0" class="health-empty">No channels match the current filter.</div>
          </div>
        </section>

        <section v-if="activeTab === 'users' && isAdmin">
          <div class="toolbar">
            <div>
              <h3>{{ editingUser ? 'Edit account' : 'Create account' }}</h3>
              <p>Disabled accounts lose console sessions, API key access, and active channel routing.</p>
            </div>
            <div class="toolbar-actions">
              <button class="ghost" @click="resetManagedUserForm">New</button>
              <button v-if="editingUser" @click="saveManagedUser">Save</button>
              <button v-else @click="createManagedUser">Create</button>
            </div>
          </div>
          <div class="form-grid compact panel">
            <label>Email <input v-model="userForm.email" autocomplete="off" /></label>
            <label>Display Name <input v-model="userForm.display_name" autocomplete="off" /></label>
            <label>Role <select v-model="userForm.role"><option value="user">User</option><option value="admin">Admin</option></select></label>
            <label>Points <input v-model.number="userForm.points_balance" type="number" step="0.0001" /></label>
            <label>Status <select v-model="userForm.enabled"><option :value="true">Enabled</option><option :value="false">Disabled</option></select></label>
            <label>Password <input v-model="userForm.password" type="password" :placeholder="editingUser ? 'Set a new password' : 'Initial password'" /></label>
          </div>
          <div v-if="editingUser" class="single-action">
            <button class="ghost" :disabled="!userForm.password" @click="resetManagedUserPassword">Reset Password</button>
          </div>
          <div class="bulkbar">
            <div class="bulk-meta">
              <strong>{{ filteredUsers.length }}</strong>
              <span>{{ users.length }} total</span>
            </div>
            <input v-model="userFilter" class="search-input" placeholder="Filter users, roles, status" />
            <button class="ghost" @click="loadUsers">Refresh</button>
          </div>
          <div class="table-shell">
            <table class="management-table user-table">
              <thead>
                <tr>
                  <th>User</th><th>Role</th><th>Status</th><th>Balance</th><th>Keys</th><th>Channels</th><th>Spent</th><th>Provided</th><th>Actions</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="record in filteredUsers" :key="record.id" :class="{ selected: editingUserId === record.id }">
                  <td>
                    <button class="link-button" @click="selectManagedUser(record)">{{ record.display_name }}</button>
                    <div class="muted-cell">#{{ record.id }} / {{ record.email }}</div>
                  </td>
                  <td><span class="scope-chip" :class="{ fallback: record.role !== 'admin' }">{{ record.role }}</span></td>
                  <td><span class="status" :class="statusClass(record)">{{ record.enabled ? 'enabled' : 'disabled' }}</span></td>
                  <td class="nowrap">{{ fmt(record.points_balance, 4) }}</td>
                  <td>{{ record.api_key_count }}</td>
                  <td>{{ record.channel_count }}</td>
                  <td class="nowrap">{{ fmt(record.total_spent_points, 4) }}</td>
                  <td class="nowrap">{{ fmt(record.total_provider_points, 4) }}</td>
                  <td>
                    <div class="row-actions compact-actions">
                      <button class="ghost" @click="selectManagedUser(record)">Edit</button>
                      <button class="ghost danger" :disabled="record.id === user.id && record.enabled" @click="toggleManagedUser(record)">{{ record.enabled ? 'Disable' : 'Enable' }}</button>
                    </div>
                  </td>
                </tr>
                <tr v-if="filteredUsers.length === 0">
                  <td colspan="9" class="empty-row">No users match the current filter.</td>
                </tr>
              </tbody>
            </table>
          </div>
        </section>

        <section v-if="activeTab === 'keys'">
          <div class="toolbar">
            <div>
              <h3>{{ editingApiKey ? 'Edit client key' : 'Issue a key' }}</h3>
              <p>Model fences, spend ceilings, rotation, and soft deletion for client credentials.</p>
            </div>
            <div class="toolbar-actions">
              <button class="ghost" @click="resetApiKeyForm">New</button>
              <button v-if="editingApiKey" @click="saveApiKey">Save</button>
              <button v-else @click="createApiKey">Create</button>
            </div>
          </div>
          <div class="form-grid compact panel">
            <label>Name <input v-model="apiKeyForm.name" /></label>
            <label>Status <select v-model="apiKeyForm.enabled"><option :value="true">Enabled</option><option :value="false">Disabled</option></select></label>
            <label>Spend Limit <input v-model.number="apiKeyForm.spend_limit_points" type="number" /></label>
            <label>Expires At <input v-model="apiKeyForm.expires_at" placeholder="2026-06-01T00:00:00Z" /></label>
            <label>Allowed Models <input v-model="apiKeyForm.allowed_models" placeholder="gpt-4o*, claude-3*" /></label>
            <div class="key-channel-field">
              <span>Allowed Channels</span>
              <button type="button" class="ghost channel-picker-trigger" @click="openApiKeyChannelModal">
                <strong>{{ channelSelectionLabel(apiKeyForm.allowed_channel_ids) }}</strong>
                <small>{{ apiKeyChannelNames(apiKeyForm.allowed_channel_ids) }}</small>
              </button>
            </div>
          </div>
          <Teleport to="body">
            <div v-if="apiKeyChannelModalOpen" class="modal-backdrop" @click.self="closeApiKeyChannelModal">
              <div class="channel-picker-modal" role="dialog" aria-modal="true" aria-label="API key channel selection">
                <div class="channel-picker-header">
                  <div>
                    <span class="section-kicker">Key channel routing</span>
                    <h3>{{ apiKeyForm.name || 'Client key' }}</h3>
                    <p>{{ channelSelectionLabel(apiKeyForm.allowed_channel_ids) }} can receive traffic from this credential.</p>
                  </div>
                  <div class="channel-picker-actions">
                    <button class="ghost" type="button" @click="selectAllApiKeyChannels">Select All</button>
                    <button class="ghost" type="button" @click="clearApiKeyChannels">Clear</button>
                    <button type="button" @click="closeApiKeyChannelModal">Done</button>
                  </div>
                </div>

                <div class="channel-picker-grid">
                  <section class="channel-picker-column" @dragover.prevent @drop="dropApiKeyChannel($event, 'available')">
                    <div class="channel-picker-column-head">
                      <div>
                        <span>Available channels</span>
                        <strong>{{ filteredAvailableApiKeyChannels.length }} / {{ availableApiKeyChannels.length }}</strong>
                      </div>
                      <button class="ghost" type="button" :disabled="filteredAvailableApiKeyChannels.length === 0" @click="selectFilteredApiKeyChannels">Add Visible</button>
                    </div>
                    <input v-model="apiKeyChannelFilter" class="search-input" placeholder="Filter provider, health, model" />
                    <div class="channel-card-list">
                      <article
                        v-for="channel in filteredAvailableApiKeyChannels"
                        :key="`available-${channel.id}`"
                        class="channel-select-card"
                        :class="{ disabled: !channel.enabled }"
                        draggable="true"
                        :title="channelCardTitle(channel)"
                        @dragstart="startApiKeyChannelDrag($event, channel.id)"
                        @dblclick="addApiKeyChannel(channel.id)"
                      >
                        <div class="channel-select-card-main">
                          <strong>{{ channel.name }}</strong>
                          <button class="ghost channel-select-action" type="button" title="Add channel" aria-label="Add channel" @click.stop="addApiKeyChannel(channel.id)">+</button>
                        </div>
                        <div class="channel-select-meta">
                          <ProviderBadge :provider="channel.provider" compact />
                          <span class="status" :class="statusClass(channel)">{{ channel.enabled ? channel.status : 'disabled' }}</span>
                          <span class="channel-select-owner">{{ ownerLabel(channel) }}</span>
                          <span class="channel-select-models">{{ (channel.models || []).join(', ') || '*' }}</span>
                        </div>
                        <HealthStrip
                          :windows="healthWindows(channel)"
                          :channel-name="channel.name"
                          :key-prefix="`${channel.id}:available-mini`"
                          variant="compact"
                          :limit="16"
                        />
                      </article>
                      <div v-if="filteredAvailableApiKeyChannels.length === 0" class="channel-picker-empty">No available channels match.</div>
                    </div>
                  </section>

                  <section class="channel-picker-column selected" @dragover.prevent @drop="dropApiKeyChannel($event, 'selected')">
                    <div class="channel-picker-column-head">
                      <div>
                        <span>Enabled for this key</span>
                        <strong>{{ filteredSelectedApiKeyChannels.length }} / {{ selectedApiKeyChannels.length }}</strong>
                      </div>
                      <button class="ghost danger" type="button" :disabled="filteredSelectedApiKeyChannels.length === 0" @click="clearFilteredApiKeyChannels">Remove Visible</button>
                    </div>
                    <input v-model="apiKeyChannelAssignedFilter" class="search-input" placeholder="Filter selected channels" />
                    <div class="channel-card-list">
                      <article
                        v-for="channel in filteredSelectedApiKeyChannels"
                        :key="`selected-${channel.id}`"
                        class="channel-select-card selected"
                        :class="{ disabled: !channel.enabled }"
                        draggable="true"
                        :title="channelCardTitle(channel)"
                        @dragstart="startApiKeyChannelDrag($event, channel.id)"
                        @dblclick="removeApiKeyChannel(channel.id)"
                      >
                        <div class="channel-select-card-main">
                          <strong>{{ channel.name }}</strong>
                          <button class="ghost danger channel-select-action" type="button" title="Remove channel" aria-label="Remove channel" @click.stop="removeApiKeyChannel(channel.id)">-</button>
                        </div>
                        <div class="channel-select-meta">
                          <ProviderBadge :provider="channel.provider" compact />
                          <span class="status" :class="statusClass(channel)">{{ channel.enabled ? channel.status : 'disabled' }}</span>
                          <span class="channel-select-owner">{{ ownerLabel(channel) }}</span>
                          <span class="channel-select-models">{{ (channel.models || []).join(', ') || '*' }}</span>
                        </div>
                        <HealthStrip
                          :windows="healthWindows(channel)"
                          :channel-name="channel.name"
                          :key-prefix="`${channel.id}:selected-mini`"
                          variant="compact"
                          :limit="16"
                        />
                      </article>
                      <div v-if="filteredSelectedApiKeyChannels.length === 0" class="channel-picker-empty">Drop channels here to authorize this key.</div>
                    </div>
                  </section>
                </div>
              </div>
            </div>
          </Teleport>
          <p v-if="newApiKey" class="secret">{{ newApiKey }}</p>
          <div class="bulkbar">
            <div class="bulk-meta">
              <strong>{{ filteredApiKeys.length }}</strong>
              <span>{{ selectedApiKeyIds.length }} selected</span>
            </div>
            <input v-model="apiKeyFilter" class="search-input" placeholder="Filter keys, prefixes, models" />
            <button class="ghost" :disabled="filteredApiKeys.length === 0" @click="toggleFilteredApiKeys">
              {{ allFilteredApiKeysSelected ? 'Clear Visible' : 'Select Visible' }}
            </button>
            <button class="ghost danger" :disabled="selectedApiKeyIds.length === 0" @click="deleteSelectedApiKeys">Delete Selected</button>
          </div>
          <div class="table-shell">
            <table class="management-table">
              <thead>
                <tr>
                  <th></th><th>Name</th><th>Prefix</th><th>Status</th><th>Spend</th><th>Models</th><th>Channels</th><th>Last Used</th><th>Actions</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="key in filteredApiKeys" :key="key.id" :class="{ selected: editingApiKeyId === key.id }">
                  <td class="select-cell"><input v-model="selectedApiKeyIds" type="checkbox" :value="key.id" /></td>
                  <td><button class="link-button" @click="selectApiKey(key)">{{ key.name }}</button></td>
                  <td><code>{{ key.key_prefix }}</code></td>
                  <td><span class="status" :class="statusClass(key)">{{ key.enabled ? 'enabled' : 'disabled' }}</span></td>
                  <td class="nowrap">{{ fmt(key.spent_points, 4) }} / {{ key.spend_limit_points ?? 'unlimited' }}</td>
                  <td class="wrap-cell">{{ (key.allowed_models || []).join(', ') || '*' }}</td>
                  <td class="wrap-cell">{{ channelSelectionLabel(key.allowed_channel_ids || []) }}</td>
                  <td class="muted-cell">{{ key.last_used_at || '-' }}</td>
                  <td>
                    <div class="row-actions">
                      <button class="ghost" @click="toggleApiKey(key)">{{ key.enabled ? 'Disable' : 'Enable' }}</button>
                      <button class="ghost" @click="rotateApiKey(key)">Rotate</button>
                      <button class="ghost danger" @click="deleteApiKey(key)">Delete</button>
                    </div>
                  </td>
                </tr>
                <tr v-if="filteredApiKeys.length === 0">
                  <td colspan="9" class="empty-row">No keys match the current filter.</td>
                </tr>
              </tbody>
            </table>
          </div>
        </section>

        <section v-if="activeTab === 'channels'">
          <div class="toolbar">
            <div>
              <h3>{{ editingChannel ? 'Edit upstream capacity' : 'Add upstream capacity' }}</h3>
              <p>{{ isAdmin ? 'Global operator view across every owner channel.' : 'Your upstream pools only; admins can still operate globally.' }}</p>
            </div>
            <div class="toolbar-actions">
              <button class="ghost" @click="resetChannelForm">New</button>
              <button v-if="editingChannel" @click="saveChannel">Save</button>
              <button v-else @click="createChannel">Add Channel</button>
            </div>
          </div>
          <div class="form-grid panel">
            <label>Name <input v-model="channelForm.name" /></label>
            <label>Provider <select v-model="channelForm.provider"><option value="openai">OpenAI</option><option value="anthropic">Anthropic</option><option value="gemini">Gemini</option></select></label>
            <label>Base URL <input v-model="channelForm.base_url" /></label>
            <label>API Key <input v-model="channelForm.api_key_secret" type="password" :placeholder="editingChannel ? 'Leave blank to keep existing key' : ''" /></label>
            <label>Models <input v-model="channelForm.models" /></label>
            <label>Status <select v-model="channelForm.enabled"><option :value="true">Enabled</option><option :value="false">Disabled</option></select></label>
            <label>Fire Sale Days <input v-model.number="channelForm.fire_sale_days_before" type="number" /></label>
            <label>Fire Sale Remaining <input v-model.number="channelForm.fire_sale_remaining_pct" type="number" step="0.01" /></label>
            <label>Fire Sale Discount <input v-model.number="channelForm.fire_sale_discount" type="number" step="0.01" /></label>
            <label>Provider Share <input v-model.number="channelForm.provider_share" type="number" step="0.01" /></label>
          </div>
          <div class="quota-editor panel">
            <div class="subtoolbar">
              <div>
                <h3>Quota windows</h3>
                <p>Every window is enforced; the first window drives inventory and fire-sale timing.</p>
              </div>
              <button class="ghost" @click="addQuotaWindow">Add Window</button>
            </div>
            <div class="quota-window" v-for="(window, index) in channelForm.windows" :key="index">
              <label>Name <input v-model="window.name" /></label>
              <label>Limit <input v-model.number="window.limit_tokens" type="number" min="1" /></label>
              <label>Every <input v-model.number="window.period_count" type="number" min="1" /></label>
              <label>Unit
                <select v-model="window.period_unit">
                  <option value="minute">Minute</option>
                  <option value="hour">Hour</option>
                  <option value="day">Day</option>
                  <option value="week">Week</option>
                  <option value="month">Month</option>
                  <option value="year">Year</option>
                </select>
              </label>
              <label>Anchor <input v-model="window.anchor_at" /></label>
              <label>Timezone <input v-model="window.timezone" /></label>
              <button class="ghost danger" :disabled="channelForm.windows.length <= 1" @click="removeQuotaWindow(index)">Remove</button>
            </div>
          </div>
          <div class="bulkbar">
            <div class="bulk-meta">
              <strong>{{ filteredChannels.length }}</strong>
              <span>{{ selectedChannelIds.length }} selected</span>
            </div>
            <input v-model="channelFilter" class="search-input" :placeholder="isAdmin ? 'Filter channels, owners, providers, models' : 'Filter channels, providers, models'" />
            <button class="ghost" :disabled="filteredChannels.length === 0" @click="toggleFilteredChannels">
              {{ allFilteredChannelsSelected ? 'Clear Visible' : 'Select Visible' }}
            </button>
            <button class="ghost" :disabled="selectedChannelIds.length === 0" @click="setSelectedChannels(true)">Enable Selected</button>
            <button class="ghost" :disabled="selectedChannelIds.length === 0" @click="setSelectedChannels(false)">Disable Selected</button>
          </div>
          <div class="table-shell">
            <table class="management-table">
              <thead>
                <tr>
                  <th></th><th>Name</th><th v-if="isAdmin">Owner</th><th>Provider</th><th>Status</th><th>Primary Left</th><th>Windows</th><th>Models</th><th>Health</th><th>Actions</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="channel in filteredChannels" :key="channel.id" :class="{ selected: editingChannelId === channel.id }">
                  <td class="select-cell"><input v-model="selectedChannelIds" type="checkbox" :value="channel.id" /></td>
                  <td><button class="link-button" @click="selectChannel(channel)">{{ channel.name }}</button></td>
                  <td v-if="isAdmin" class="muted-cell">{{ ownerLabel(channel) }}</td>
                  <td>{{ channel.provider }}</td>
                  <td><span class="status" :class="statusClass(channel)">{{ channel.status }}</span></td>
                  <td class="nowrap">{{ primaryWindow(channel) ? fmt(primaryWindow(channel).limit_tokens - primaryWindow(channel).used_tokens, 0) : '-' }}</td>
                  <td class="wrap-cell">{{ quotaSummary(channel) }}</td>
                  <td class="wrap-cell">{{ channel.models.join(', ') || '*' }}</td>
                  <td class="health-cell">
                    <div class="health-summary">
                      <strong :class="`tone-${healthSummary(channel).tone}`">{{ healthSummary(channel).label }}</strong>
                      <span>{{ healthSummary(channel).detail }}</span>
                    </div>
                    <HealthStrip
                      :windows="healthWindows(channel)"
                      :channel-name="channel.name"
                      :key-prefix="`${channel.id}`"
                    />
                  </td>
                  <td>
                    <div class="row-actions">
                      <button class="ghost" @click="toggleChannel(channel)">{{ channel.enabled ? 'Disable' : 'Enable' }}</button>
                      <button class="ghost" @click="testChannel(channel)">Test</button>
                      <button class="ghost" @click="copyChannel(channel)">Copy</button>
                      <button class="ghost danger" @click="deleteChannel(channel)">Delete</button>
                    </div>
                  </td>
                </tr>
                <tr v-if="filteredChannels.length === 0">
                  <td :colspan="isAdmin ? 10 : 9" class="empty-row">No channels match the current filter.</td>
                </tr>
              </tbody>
            </table>
          </div>
        </section>

        <section v-if="activeTab === 'prices'">
          <div class="toolbar">
            <div>
              <h3>Settle model cost</h3>
              <p>{{ isAdmin ? 'Channel prices override global model defaults.' : 'Your channel prices override the visible default fallback rows.' }}</p>
            </div>
            <button :disabled="priceSaveDisabled" @click="savePrice">Save</button>
          </div>
          <div class="form-grid compact panel">
            <label>Preset <select v-model="selectedPricePreset" @change="applyPricePreset"><option value="">Custom</option><option v-for="preset in pricePresets" :key="preset.key" :value="preset.key">{{ preset.label }}</option></select></label>
            <label>Scope <select v-model="priceForm.channel_id" :disabled="!isAdmin && channels.length === 0"><option v-if="isAdmin" :value="null">Global default</option><option v-for="channel in channels" :key="channel.id" :value="channel.id">{{ channelOptionLabel(channel) }}</option></select></label>
            <label>Model Pattern <input v-model="priceForm.model_pattern" /></label>
            <label>Input / 1M <input v-model.number="priceForm.input_price_per_1m" type="number" step="0.01" /></label>
            <label>Output / 1M <input v-model.number="priceForm.output_price_per_1m" type="number" step="0.01" /></label>
            <label>Cache / 1M <input v-model.number="priceForm.cache_price_per_1m" type="number" step="0.01" /></label>
          </div>
          <div class="table-shell">
            <table>
              <thead>
                <tr><th>Scope</th><th v-if="isAdmin">Owner</th><th>Model</th><th>Input / 1M</th><th>Output / 1M</th><th>Cache / 1M</th></tr>
              </thead>
              <tbody>
                <tr v-for="price in prices" :key="`${price.channel_id || 'global'}:${price.model_pattern}`" :class="{ 'muted-row': !isAdmin && isGlobalPrice(price) }">
                  <td><span class="scope-chip" :class="{ fallback: isGlobalPrice(price) }">{{ priceScope(price) }}</span></td>
                  <td v-if="isAdmin" class="muted-cell">{{ priceOwnerLabel(price) }}</td>
                  <td>{{ price.model_pattern }}</td>
                  <td>{{ price.input_price_per_1m }}</td>
                  <td>{{ price.output_price_per_1m }}</td>
                  <td>{{ price.cache_price_per_1m }}</td>
                </tr>
                <tr v-if="prices.length === 0">
                  <td :colspan="isAdmin ? 6 : 5" class="empty-row">No pricing rules configured.</td>
                </tr>
              </tbody>
            </table>
          </div>
        </section>

        <section v-if="activeTab === 'affinity'">
          <div class="toolbar"><div><h3>Bind traffic lanes</h3><p>Sticky channel bindings for tenants, sessions, and prompt-cache locality.</p></div><button :disabled="!isAdmin" @click="createRule">Create</button></div>
          <div class="form-grid compact panel">
            <label>Name <input v-model="ruleForm.name" /></label>
            <label>Path <input v-model="ruleForm.request_path" /></label>
            <label>Model Regex <input v-model="ruleForm.model_regex" /></label>
            <label>Source <select v-model="ruleForm.key_source_type"><option value="request_header">Header</option><option value="json_path">JSON Path</option><option value="context">Context</option></select></label>
            <label>Source Path <input v-model="ruleForm.key_source_path" /></label>
            <label>TTL <input v-model.number="ruleForm.ttl_seconds" type="number" /></label>
            <label><input v-model="ruleForm.include_model_name" type="checkbox" /> Model-scoped key</label>
          </div>
          <div class="table-shell">
            <table><tbody><tr v-for="rule in rules" :key="rule.id"><td>{{ rule.name }}</td><td>{{ rule.request_path }}</td><td>{{ rule.key_source_type }}:{{ rule.key_source_path }}</td><td>{{ rule.ttl_seconds }}s</td></tr></tbody></table>
          </div>
        </section>

        <section v-if="activeTab === 'economy'">
          <div class="toolbar"><div><h3>Move value</h3><p>P2P transfers and phrase red packets.</p></div><button class="ghost" @click="refreshAll">Refresh</button></div>
          <div class="two-col">
            <article class="panel">
              <h3>P2P Transfer</h3>
              <div class="form-stack">
                <label>Recipient User ID <input v-model.number="transferForm.to_user_id" type="number" /></label>
                <label>Points <input v-model.number="transferForm.points" type="number" step="0.0001" /></label>
                <label>Memo <input v-model="transferForm.memo" /></label>
                <button @click="transferPoints">Transfer</button>
              </div>
            </article>
            <article class="panel">
              <h3>Red Packet</h3>
              <div class="form-stack">
                <label>Phrase <input v-model="redPacketForm.phrase" /></label>
                <label>Total Points <input v-model.number="redPacketForm.total_points" type="number" /></label>
                <label>Parts <input v-model.number="redPacketForm.total_parts" type="number" /></label>
                <label>Mode <select v-model="redPacketForm.mode"><option value="even">Even</option><option value="lucky">Lucky</option></select></label>
                <button @click="createRedPacket">Create</button>
              </div>
            </article>
            <article class="panel">
              <h3>Claim Phrase</h3>
              <div class="form-stack">
                <label>Phrase <input v-model="claimForm.phrase" /></label>
                <button @click="claimRedPacket">Claim</button>
                <p v-if="claimResult" class="secret">{{ claimResult }}</p>
              </div>
            </article>
            <article class="panel">
              <h3>Anonymous Ranking</h3>
              <p class="muted">Current: {{ user.anonymous_leaderboard ? 'Anonymous' : 'Public' }}</p>
              <button class="ghost" @click="toggleAnonymous">Toggle</button>
            </article>
          </div>
          <div class="economy-history">
            <article class="panel history-panel">
              <header class="history-head">
                <div>
                  <span>Recent flows</span>
                  <h3>Transfers</h3>
                </div>
                <strong>{{ transfers.length }}</strong>
              </header>
              <div v-if="transfers.length" class="history-list">
                <article
                  v-for="item in transfers"
                  :key="item.id"
                  class="history-row transfer-row"
                  :class="{ incoming: item.to_user_id === user.id, outgoing: item.to_user_id !== user.id }"
                >
                  <div class="history-main">
                    <span class="direction-pill">{{ transferDirection(item) }}</span>
                    <strong>{{ item.from_name }} <span aria-hidden="true">-&gt;</span> {{ item.to_name }}</strong>
                    <small>{{ item.memo || 'No memo' }}</small>
                  </div>
                  <div class="history-meta">
                    <strong>{{ signedTransferPoints(item) }}</strong>
                    <small>{{ compactDate(item.created_at) }}</small>
                  </div>
                </article>
              </div>
              <div v-else class="history-empty">
                <strong>No transfers</strong>
                <span>Recent point movements will appear here.</span>
              </div>
            </article>

            <article class="panel history-panel">
              <header class="history-head">
                <div>
                  <span>Phrase packets</span>
                  <h3>My Red Packets</h3>
                </div>
                <strong>{{ redPackets.length }}</strong>
              </header>
              <div v-if="redPackets.length" class="packet-list">
                <article v-for="packet in redPackets" :key="packet.id" class="packet-card">
                  <div class="packet-title">
                    <strong>{{ packet.phrase }}</strong>
                    <span class="mode-chip">{{ packet.mode }}</span>
                  </div>
                  <div class="packet-progress" :aria-label="`${packetClaimedPct(packet)}% claimed`">
                    <span :style="{ width: `${packetClaimedPct(packet)}%` }"></span>
                  </div>
                  <div class="packet-stats">
                    <span>
                      <b>{{ packet.claimed_parts }}/{{ packet.total_parts }}</b>
                      <small>claimed</small>
                    </span>
                    <span>
                      <b>{{ fmt(packet.remaining_points, 4) }}</b>
                      <small>remaining</small>
                    </span>
                    <span>
                      <b>{{ compactDate(packet.created_at) }}</b>
                      <small>created</small>
                    </span>
                  </div>
                </article>
              </div>
              <div v-else class="history-empty">
                <strong>No packets</strong>
                <span>Created red packets will appear here.</span>
              </div>
            </article>
          </div>
        </section>

        <section v-if="activeTab === 'leaderboards'">
          <div class="leaderboard-hero">
            <div>
              <span class="section-kicker">{{ leaderboardPeriod === 'day' ? 'Daily Circuit' : 'Monthly Circuit' }}</span>
              <h3>{{ leaderboardPeriod === 'day' ? 'Today on the altar' : 'This month on the altar' }}</h3>
              <p>Provider token output and consumer point burn share the same settlement window.</p>
            </div>
            <div class="leaderboard-controls">
              <div class="segmented small">
                <button :class="{ active: leaderboardPeriod === 'day' }" @click="setLeaderboardPeriod('day')">Day</button>
                <button :class="{ active: leaderboardPeriod === 'month' }" @click="setLeaderboardPeriod('month')">Month</button>
              </div>
              <button class="ghost" @click="loadLeaderboards">Refresh</button>
            </div>
          </div>

          <div class="leaderboard-meta-grid">
            <article v-for="item in leaderboardSummary" :key="item.label">
              <span>{{ item.label }}</span>
              <strong>{{ item.value }}</strong>
              <em>{{ item.detail }}</em>
            </article>
            <article>
              <span>Window start</span>
              <strong>{{ leaderboards.window_start || '-' }}</strong>
              <em>{{ leaderboards.timezone || 'server-local' }}</em>
            </article>
          </div>

          <div class="leaderboard-grid">
            <article class="leaderboard-board provider-board">
              <div class="leaderboard-board-header">
                <div>
                  <span>Providers</span>
                  <h3>Token supply</h3>
                </div>
                <strong>{{ providerRows.length }}</strong>
              </div>
              <div v-if="providerRows.length" class="leaderboard-list">
                <div v-for="row in providerRows" :key="row.key" class="leaderboard-entry" :class="`tone-${row.tone}`">
                  <div class="rank-badge">{{ row.rank }}</div>
                  <div class="leaderboard-person">
                    <strong>{{ row.name }}</strong>
                    <span>{{ row.user_id ? `User #${row.user_id}` : 'Anonymous steward' }}</span>
                  </div>
                  <div class="leaderboard-score">
                    <strong>{{ row.scoreText }}</strong>
                    <span class="score-track"><i :style="{ width: `${row.share}%` }"></i></span>
                  </div>
                </div>
              </div>
              <div v-else class="leaderboard-empty">No provider settlements in this window.</div>
            </article>

            <article class="leaderboard-board consumer-board">
              <div class="leaderboard-board-header">
                <div>
                  <span>Consumers</span>
                  <h3>Point burn</h3>
                </div>
                <strong>{{ consumerRows.length }}</strong>
              </div>
              <div v-if="consumerRows.length" class="leaderboard-list">
                <div v-for="row in consumerRows" :key="row.key" class="leaderboard-entry" :class="`tone-${row.tone}`">
                  <div class="rank-badge">{{ row.rank }}</div>
                  <div class="leaderboard-person">
                    <strong>{{ row.name }}</strong>
                    <span>{{ row.user_id ? `User #${row.user_id}` : 'Anonymous steward' }}</span>
                  </div>
                  <div class="leaderboard-score">
                    <strong>{{ row.scoreText }}</strong>
                    <span class="score-track"><i :style="{ width: `${row.share}%` }"></i></span>
                  </div>
                </div>
              </div>
              <div v-else class="leaderboard-empty">No consumer settlements in this window.</div>
            </article>
          </div>
        </section>

        <section v-if="activeTab === 'ledger'">
          <div class="toolbar"><div><h3>Usage archive</h3><p>Input, output, cache tokens and settlement formula.</p></div><button class="ghost" @click="loadLedger">Refresh</button></div>
          <div class="table-shell">
            <table><tbody><tr v-for="entry in ledger" :key="entry.id"><td>{{ entry.created_at }}</td><td>{{ entry.model }}</td><td>{{ entry.input_tokens }}/{{ entry.output_tokens }}/{{ entry.cache_tokens }}</td><td>{{ fmt(entry.total_points, 4) }}</td><td>{{ entry.tokenizer }}</td><td>{{ entry.formula_note }}</td></tr></tbody></table>
          </div>
        </section>

        <section v-if="activeTab === 'guide'" class="guide-page">
          <a class="guide-frame" href="/guides/tokenaltar-project-guide.png" target="_blank" rel="noreferrer">
            <img src="/guides/tokenaltar-project-guide.png" alt="TokenAltar project guide relief" />
          </a>
          <section class="guide-codex" aria-labelledby="guide-codex-title">
            <div class="guide-codex-hero">
              <div>
                <span class="section-kicker">Living Rulebook</span>
                <h3 id="guide-codex-title">Mechanisms beneath the relief</h3>
                <p>{{ guideCanonIntro }}</p>
              </div>
            </div>
            <div class="guide-pillars">
              <article v-for="item in guideMechanisms" :key="item.index" class="guide-pillar-card">
                <div class="pillar-capital" aria-hidden="true">
                  <span>{{ item.index }}</span>
                </div>
                <div class="guide-pillar-body">
                  <span class="guide-badge">{{ item.badge }}</span>
                  <h4>{{ item.title }}</h4>
                  <p>{{ item.summary }}</p>
                  <ul>
                    <li v-for="point in item.points" :key="point">{{ point }}</li>
                  </ul>
                </div>
              </article>
            </div>
          </section>
        </section>

        <section v-if="activeTab === 'settings' && isAdmin">
          <div class="toolbar"><div><h3>Runtime controls</h3><p>Gateway economy, routing, capacity, and console defaults.</p></div><button @click="saveSettings">Save</button></div>
          <div class="form-grid panel settings-grid">
            <label v-for="item in settingsSchema" :key="item.key" :class="{ wide: item.type === 'textarea' }">
              {{ item.label }}
              <select v-if="item.type === 'boolean'" v-model="settingsForm[item.key]">
                <option value="false">false</option>
                <option value="true">true</option>
              </select>
              <textarea v-else-if="item.type === 'textarea'" v-model="settingsForm[item.key]" rows="5"></textarea>
              <input v-else v-model="settingsForm[item.key]" :type="item.type" :step="item.type === 'number' ? 'any' : undefined" />
            </label>
          </div>
          <div class="table-shell">
            <table><tbody><tr v-for="setting in settings" :key="setting.key"><td>{{ setting.key }}</td><td>{{ setting.value }}</td><td>{{ setting.updated_at }}</td></tr></tbody></table>
          </div>
        </section>
      </template>
    </section>
  </main>
</template>
