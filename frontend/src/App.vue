<script setup lang="ts">
import { computed, onMounted, reactive, ref } from 'vue'

type User = {
  id: number
  email: string
  role: string
  display_name: string
  points_balance: number
  anonymous_leaderboard: boolean
}

type Dashboard = {
  users: number
  channels: number
  enabled_channels: number
  available_tokens: number
  spent_points_today: number
  surge_multiplier: number
  surge_state: string
}

type TabId =
  | 'dashboard'
  | 'keys'
  | 'channels'
  | 'prices'
  | 'affinity'
  | 'economy'
  | 'leaderboards'
  | 'ledger'
  | 'settings'

type TabItem = [TabId, string]

const token = ref(localStorage.getItem('tokenaltar_token') || '')
const user = ref<User | null>(null)
const error = ref('')
const activeTab = ref<TabId>('dashboard')
const authMode = ref<'login' | 'register'>('login')
const apiKeys = ref<any[]>([])
const channels = ref<any[]>([])
const prices = ref<any[]>([])
const rules = ref<any[]>([])
const ledger = ref<any[]>([])
const transfers = ref<any[]>([])
const redPackets = ref<any[]>([])
const leaderboards = ref<any>({ providers: [], consumers: [] })
const leaderboardPeriod = ref<'day' | 'month'>('month')
const settings = ref<any[]>([])
const dashboard = ref<Dashboard | null>(null)
const newApiKey = ref('')
const claimResult = ref('')
const selectedApiKeyIds = ref<number[]>([])
const selectedChannelIds = ref<number[]>([])
const editingApiKeyId = ref<number | null>(null)
const editingChannelId = ref<number | null>(null)
const channelTestResults = ref<Record<number, string>>({})
const apiKeyFilter = ref('')
const channelFilter = ref('')

const loginForm = reactive({ email: 'admin@example.com', password: '' })
const registerForm = reactive({ email: '', password: '', display_name: '', invite_code: '' })
const apiKeyForm = reactive({
  name: 'local-dev',
  spend_limit_points: 1000 as number | null,
  enabled: true,
  expires_at: '',
  allowed_models: '',
})
const channelForm = reactive({
  name: 'OpenAI Pool',
  provider: 'openai',
  base_url: 'https://api.openai.com',
  api_key_secret: '',
  models: 'gpt-*,gpt-4o*',
  enabled: true,
  cycle_limit_tokens: 1000000,
  cycle_reset_day: 1,
  daily_limit_tokens: 200000,
  hourly_limit_tokens: 50000,
  fire_sale_days_before: 3,
  fire_sale_remaining_pct: 0.25,
  fire_sale_discount: 0.2,
  provider_share: 0.7,
})
const priceForm = reactive({
  channel_id: null as number | null,
  model_pattern: 'default',
  input_price_per_1k: 1,
  output_price_per_1k: 3,
  cache_price_per_1k: 0.2,
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
})
const transferForm = reactive({ to_user_id: 0, points: 10, memo: '@TokenAltar PayTo:' })
const redPacketForm = reactive({ phrase: 'RustIsBest', total_points: 30, total_parts: 3, mode: 'even' })
const claimForm = reactive({ phrase: 'RustIsBest' })
const settingsForm = reactive({ invite_required: 'false', invite_code_default: 'TOKENALTAR' })

const isAdmin = computed(() => user.value?.role === 'admin')
const tabDetails: Record<TabId, { eyebrow: string; title: string; description: string }> = {
  dashboard: {
    eyebrow: 'Capacity Atrium',
    title: 'Gateway dashboard',
    description: 'Live token supply, surge pressure, and the service routes exposed to clients.',
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
  settings: {
    eyebrow: 'Admin Chamber',
    title: 'Console settings',
    description: 'Local invite controls for a gated TokenAltar circle.',
  },
}
const tabs = computed<TabItem[]>(() => {
  const items: TabItem[] = [
    ['dashboard', 'Dashboard'],
    ['keys', 'API Keys'],
    ['channels', 'Channels'],
    ['prices', 'Pricing'],
    ['affinity', 'Affinity'],
    ['economy', 'Economy'],
    ['leaderboards', 'Leaderboards'],
    ['ledger', 'Ledger'],
  ]
  if (isAdmin.value) items.push(['settings', 'Settings'])
  return items
})
const activeTabMeta = computed(() => tabDetails[activeTab.value])
const editingApiKey = computed(() => apiKeys.value.find((item) => item.id === editingApiKeyId.value))
const editingChannel = computed(() => channels.value.find((item) => item.id === editingChannelId.value))
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
const filteredChannels = computed(() => {
  const needle = channelFilter.value.trim().toLowerCase()
  if (!needle) return channels.value
  return channels.value.filter((channel) => {
    const haystack = [
      channel.name,
      channel.provider,
      channel.status,
      channel.base_url,
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
  { label: 'Surge', value: dashboard.value?.surge_state || 'idle', detail: `${dashboard.value?.surge_multiplier || 1}x multiplier` },
  { label: 'Available tokens', value: fmt(dashboard.value?.available_tokens, 0), detail: 'ready for routing' },
  { label: 'Enabled channels', value: `${dashboard.value?.enabled_channels || 0} / ${dashboard.value?.channels || 0}`, detail: 'online capacity' },
  { label: 'Today spend', value: fmt(dashboard.value?.spent_points_today, 4), detail: 'points settled' },
])

async function api(path: string, options: RequestInit = {}) {
  error.value = ''
  const response = await fetch(`/api${path}`, {
    ...options,
    headers: {
      'content-type': 'application/json',
      ...(token.value ? { authorization: `Bearer ${token.value}` } : {}),
      ...(options.headers || {}),
    },
  })
  const text = await response.text()
  const data = text ? JSON.parse(text) : null
  if (!response.ok) throw new Error(data?.error || response.statusText)
  return data
}

async function login() {
  try {
    const data = await api('/auth/login', { method: 'POST', body: JSON.stringify(loginForm) })
    acceptAuth(data)
  } catch (err) {
    error.value = String(err)
  }
}

async function register() {
  try {
    const data = await api('/auth/register', { method: 'POST', body: JSON.stringify(registerForm) })
    acceptAuth(data)
  } catch (err) {
    error.value = String(err)
  }
}

async function acceptAuth(data: any) {
  token.value = data.token
  user.value = data.user
  localStorage.setItem('tokenaltar_token', data.token)
  await refreshAll()
}

function logout() {
  token.value = ''
  user.value = null
  localStorage.removeItem('tokenaltar_token')
}

async function refreshAll() {
  if (!token.value) return
  try {
    user.value = await api('/me')
    await Promise.all([
      loadDashboard(),
      loadApiKeys(),
      loadChannels(),
      loadPrices(),
      loadRules(),
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

async function loadDashboard() { dashboard.value = await api('/dashboard') }
async function loadApiKeys() { apiKeys.value = await api('/api-keys') }
async function loadChannels() {
  channels.value = await api('/channels')
  if (!isAdmin.value && !priceForm.channel_id && channels.value.length > 0) {
    priceForm.channel_id = channels.value[0].id
  }
}
async function loadPrices() { prices.value = await api('/prices') }
async function loadRules() { rules.value = await api('/affinity-rules') }
async function loadLedger() { ledger.value = await api('/ledger') }
async function loadTransfers() { transfers.value = await api('/transfers') }
async function loadRedPackets() { redPackets.value = await api('/red-packets') }
async function loadLeaderboards() { leaderboards.value = await api(`/leaderboards?period=${leaderboardPeriod.value}`) }

async function loadSettings() {
  settings.value = await api('/settings')
  for (const setting of settings.value) {
    if (setting.key in settingsForm) {
      ;(settingsForm as any)[setting.key] = setting.value
    }
  }
}

async function createApiKey() {
  const data = await api('/api-keys', {
    method: 'POST',
    body: JSON.stringify(apiKeyPayload()),
  })
  newApiKey.value = data.token
  editingApiKeyId.value = data.record.id
  await loadApiKeys()
}

async function toggleApiKey(record: any) {
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

async function rotateApiKey(record: any) {
  const data = await api(`/api-keys/${record.id}/rotate`, { method: 'POST' })
  newApiKey.value = data.token
  editingApiKeyId.value = record.id
  await loadApiKeys()
}

async function deleteApiKey(record: any) {
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

function selectApiKey(record: any) {
  editingApiKeyId.value = record.id
  apiKeyForm.name = record.name
  apiKeyForm.spend_limit_points = record.spend_limit_points
  apiKeyForm.enabled = record.enabled
  apiKeyForm.expires_at = record.expires_at || ''
  apiKeyForm.allowed_models = (record.allowed_models || []).join(', ')
}

function resetApiKeyForm() {
  editingApiKeyId.value = null
  apiKeyForm.name = 'local-dev'
  apiKeyForm.spend_limit_points = 1000
  apiKeyForm.enabled = true
  apiKeyForm.expires_at = ''
  apiKeyForm.allowed_models = ''
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

async function toggleChannel(channel: any) {
  await api(`/channels/${channel.id}/enabled`, {
    method: 'POST',
    body: JSON.stringify({ enabled: !channel.enabled }),
  })
  await Promise.all([loadChannels(), loadDashboard()])
}

async function deleteChannel(channel: any) {
  await api(`/channels/${channel.id}`, { method: 'DELETE' })
  selectedChannelIds.value = selectedChannelIds.value.filter((id) => id !== channel.id)
  if (editingChannelId.value === channel.id) resetChannelForm()
  await Promise.all([loadChannels(), loadDashboard()])
}

async function copyChannel(channel: any) {
  const copied = await api(`/channels/${channel.id}/copy`, {
    method: 'POST',
    body: JSON.stringify({ suffix: ' copy', reset_usage: true }),
  })
  await Promise.all([loadChannels(), loadDashboard()])
  selectChannel(copied)
}

async function testChannel(channel: any) {
  channelTestResults.value[channel.id] = 'testing...'
  const result = await api(`/channels/${channel.id}/test`, { method: 'POST' })
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

function selectChannel(channel: any) {
  editingChannelId.value = channel.id
  channelForm.name = channel.name
  channelForm.provider = channel.provider
  channelForm.base_url = channel.base_url
  channelForm.api_key_secret = ''
  channelForm.models = (channel.models || []).join(', ')
  channelForm.enabled = channel.enabled
  channelForm.cycle_limit_tokens = channel.limits.cycle_limit_tokens
  channelForm.cycle_reset_day = channel.limits.cycle_reset_day
  channelForm.daily_limit_tokens = channel.limits.daily_limit_tokens
  channelForm.hourly_limit_tokens = channel.limits.hourly_limit_tokens
  channelForm.fire_sale_days_before = channel.limits.fire_sale_days_before
  channelForm.fire_sale_remaining_pct = channel.limits.fire_sale_remaining_pct
  channelForm.fire_sale_discount = channel.limits.fire_sale_discount
  channelForm.provider_share = channel.limits.provider_share
}

function resetChannelForm() {
  editingChannelId.value = null
  channelForm.name = 'OpenAI Pool'
  channelForm.provider = 'openai'
  channelForm.base_url = 'https://api.openai.com'
  channelForm.api_key_secret = ''
  channelForm.models = 'gpt-*,gpt-4o*'
  channelForm.enabled = true
  channelForm.cycle_limit_tokens = 1000000
  channelForm.cycle_reset_day = 1
  channelForm.daily_limit_tokens = 200000
  channelForm.hourly_limit_tokens = 50000
  channelForm.fire_sale_days_before = 3
  channelForm.fire_sale_remaining_pct = 0.25
  channelForm.fire_sale_discount = 0.2
  channelForm.provider_share = 0.7
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
  await api('/prices', { method: 'POST', body: JSON.stringify(priceForm) })
  await loadPrices()
}

function priceScope(price: any) {
  if (!price.channel_id) return 'Global'
  return channels.value.find((channel) => channel.id === price.channel_id)?.name || `Channel #${price.channel_id}`
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
  const data = await api('/red-packets/claim', { method: 'POST', body: JSON.stringify(claimForm) })
  claimResult.value = `Claimed ${data.points.toFixed(4)} points`
  await Promise.all([loadRedPackets(), refreshMe()])
}

async function toggleAnonymous() {
  const updated = await api('/profile/anonymous-leaderboard', {
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
    body: JSON.stringify([
      { key: 'invite_required', value: settingsForm.invite_required },
      { key: 'invite_code_default', value: settingsForm.invite_code_default },
    ]),
  })
  await loadSettings()
}

async function refreshMe() {
  user.value = await api('/me')
}

function fmt(value: number | undefined, digits = 2) {
  return Number(value || 0).toLocaleString(undefined, { maximumFractionDigits: digits })
}

function apiKeyPayload() {
  return {
    name: apiKeyForm.name,
    enabled: apiKeyForm.enabled,
    spend_limit_points: optionalNumber(apiKeyForm.spend_limit_points),
    expires_at: apiKeyForm.expires_at || null,
    allowed_models: splitCsv(apiKeyForm.allowed_models),
  }
}

function channelPayload() {
  return {
    ...channelForm,
    api_key_secret: channelForm.api_key_secret || null,
    models: splitCsv(channelForm.models),
  }
}

function splitCsv(value: string) {
  return value.split(',').map((item) => item.trim()).filter(Boolean)
}

function optionalNumber(value: number | string | null) {
  if (value === null || value === '') return null
  const parsed = Number(value)
  return Number.isFinite(parsed) ? parsed : null
}

function healthLabel(channel: any) {
  if (channelTestResults.value[channel.id]) return channelTestResults.value[channel.id]
  if (channel.last_error) return channel.last_error
  if (channel.upstream_latency_ms) return `${channel.upstream_latency_ms}ms`
  if (channel.health_checked_at) return `checked ${channel.health_checked_at}`
  return '-'
}

function statusClass(record: any) {
  return {
    off: !record.enabled || record.status === 'manual_disabled',
    warn: record.status === 'cooling',
    danger: record.status === 'monthly_exhausted' || record.status === 'deleted',
  }
}

onMounted(refreshAll)
</script>

<template>
  <main class="shell">
    <aside class="sidebar">
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
            <span>Private token exchange</span>
            <h2>Token<wbr />Altar</h2>
            <p>Pool upstream capacity, meter token cost, and settle a small-circle LLM economy from one console.</p>
          </div>
          <div class="auth-card">
            <div class="segmented">
              <button :class="{ active: authMode === 'login' }" @click="authMode = 'login'">Login</button>
              <button :class="{ active: authMode === 'register' }" @click="authMode = 'register'">Register</button>
            </div>
            <template v-if="authMode === 'login'">
              <div class="card-heading">
                <span>Console Access</span>
                <h3>Sign in</h3>
              </div>
              <label>Email <input v-model="loginForm.email" autocomplete="username" /></label>
              <label>Password <input v-model="loginForm.password" type="password" autocomplete="current-password" /></label>
              <button @click="login">Enter console</button>
            </template>
            <template v-else>
              <div class="card-heading">
                <span>New Steward</span>
                <h3>Create account</h3>
              </div>
              <label>Email <input v-model="registerForm.email" /></label>
              <label>Name <input v-model="registerForm.display_name" /></label>
              <label>Password <input v-model="registerForm.password" type="password" /></label>
              <label>Invite Code <input v-model="registerForm.invite_code" /></label>
              <button @click="register">Register</button>
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
          </div>
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
                  <th></th><th>Name</th><th>Prefix</th><th>Status</th><th>Spend</th><th>Models</th><th>Last Used</th><th>Actions</th>
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
                  <td colspan="8" class="empty-row">No keys match the current filter.</td>
                </tr>
              </tbody>
            </table>
          </div>
        </section>

        <section v-if="activeTab === 'channels'">
          <div class="toolbar">
            <div>
              <h3>{{ editingChannel ? 'Edit upstream capacity' : 'Add upstream capacity' }}</h3>
              <p>Lifecycle controls mirror production gateways: edit, test, clone, disable, and retire channels.</p>
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
            <label>Reset Day <input v-model.number="channelForm.cycle_reset_day" type="number" min="1" max="28" /></label>
            <label>Cycle Limit <input v-model.number="channelForm.cycle_limit_tokens" type="number" /></label>
            <label>Daily Limit <input v-model.number="channelForm.daily_limit_tokens" type="number" /></label>
            <label>Hourly Limit <input v-model.number="channelForm.hourly_limit_tokens" type="number" /></label>
            <label>Fire Sale Days <input v-model.number="channelForm.fire_sale_days_before" type="number" /></label>
            <label>Fire Sale Remaining <input v-model.number="channelForm.fire_sale_remaining_pct" type="number" step="0.01" /></label>
            <label>Fire Sale Discount <input v-model.number="channelForm.fire_sale_discount" type="number" step="0.01" /></label>
            <label>Provider Share <input v-model.number="channelForm.provider_share" type="number" step="0.01" /></label>
          </div>
          <div class="bulkbar">
            <div class="bulk-meta">
              <strong>{{ filteredChannels.length }}</strong>
              <span>{{ selectedChannelIds.length }} selected</span>
            </div>
            <input v-model="channelFilter" class="search-input" placeholder="Filter channels, providers, models" />
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
                  <th></th><th>Name</th><th>Provider</th><th>Status</th><th>Cycle Left</th><th>Day/Hour</th><th>Models</th><th>Health</th><th>Actions</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="channel in filteredChannels" :key="channel.id" :class="{ selected: editingChannelId === channel.id }">
                  <td class="select-cell"><input v-model="selectedChannelIds" type="checkbox" :value="channel.id" /></td>
                  <td><button class="link-button" @click="selectChannel(channel)">{{ channel.name }}</button></td>
                  <td>{{ channel.provider }}</td>
                  <td><span class="status" :class="statusClass(channel)">{{ channel.status }}</span></td>
                  <td class="nowrap">{{ fmt(channel.limits.cycle_limit_tokens - channel.limits.used_cycle_tokens, 0) }}</td>
                  <td class="nowrap">{{ fmt(channel.limits.daily_limit_tokens - channel.limits.used_day_tokens, 0) }} / {{ fmt(channel.limits.hourly_limit_tokens - channel.limits.used_hour_tokens, 0) }}</td>
                  <td class="wrap-cell">{{ channel.models.join(', ') || '*' }}</td>
                  <td class="wrap-cell">{{ healthLabel(channel) }}</td>
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
                  <td colspan="9" class="empty-row">No channels match the current filter.</td>
                </tr>
              </tbody>
            </table>
          </div>
        </section>

        <section v-if="activeTab === 'prices'">
          <div class="toolbar"><div><h3>Settle model cost</h3><p>Channel prices override global model defaults.</p></div><button @click="savePrice">Save</button></div>
          <div class="form-grid compact panel">
            <label>Scope <select v-model="priceForm.channel_id"><option v-if="isAdmin" :value="null">Global default</option><option v-for="channel in channels" :key="channel.id" :value="channel.id">{{ channel.name }}</option></select></label>
            <label>Model Pattern <input v-model="priceForm.model_pattern" /></label>
            <label>Input / 1k <input v-model.number="priceForm.input_price_per_1k" type="number" step="0.01" /></label>
            <label>Output / 1k <input v-model.number="priceForm.output_price_per_1k" type="number" step="0.01" /></label>
            <label>Cache / 1k <input v-model.number="priceForm.cache_price_per_1k" type="number" step="0.01" /></label>
          </div>
          <div class="table-shell">
            <table><tbody><tr v-for="price in prices" :key="`${price.channel_id || 'global'}:${price.model_pattern}`"><td>{{ priceScope(price) }}</td><td>{{ price.model_pattern }}</td><td>{{ price.input_price_per_1k }}</td><td>{{ price.output_price_per_1k }}</td><td>{{ price.cache_price_per_1k }}</td></tr></tbody></table>
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
          <div class="table-pair">
            <div class="table-shell"><table><caption>Transfers</caption><tbody><tr v-for="item in transfers" :key="item.id"><td>{{ item.from_name }} -> {{ item.to_name }}</td><td>{{ fmt(item.points, 4) }}</td><td>{{ item.memo }}</td></tr></tbody></table></div>
            <div class="table-shell"><table><caption>My Red Packets</caption><tbody><tr v-for="packet in redPackets" :key="packet.id"><td>{{ packet.phrase }}</td><td>{{ packet.mode }}</td><td>{{ packet.claimed_parts }}/{{ packet.total_parts }}</td><td>{{ fmt(packet.remaining_points, 4) }}</td></tr></tbody></table></div>
          </div>
        </section>

        <section v-if="activeTab === 'leaderboards'">
          <div class="toolbar">
            <div><h3>{{ leaderboardPeriod === 'day' ? 'Daily honors' : 'Monthly honors' }}</h3><p>Provider tokens and consumer point burn.</p></div>
            <div class="toolbar-actions">
              <div class="segmented small">
                <button :class="{ active: leaderboardPeriod === 'day' }" @click="setLeaderboardPeriod('day')">Day</button>
                <button :class="{ active: leaderboardPeriod === 'month' }" @click="setLeaderboardPeriod('month')">Month</button>
              </div>
              <button class="ghost" @click="loadLeaderboards">Refresh</button>
            </div>
          </div>
          <p class="muted leaderboard-window">Window starts {{ leaderboards.window_start || '-' }} / {{ leaderboards.timezone || 'server-local' }}</p>
          <div class="table-pair">
            <div class="table-shell"><table><caption>Providers</caption><tbody><tr v-for="row in leaderboards.providers" :key="row.name"><td>{{ row.name }}</td><td>{{ fmt(row.score, 0) }} tokens</td></tr></tbody></table></div>
            <div class="table-shell"><table><caption>Consumers</caption><tbody><tr v-for="row in leaderboards.consumers" :key="row.name"><td>{{ row.name }}</td><td>{{ fmt(row.score, 4) }} points</td></tr></tbody></table></div>
          </div>
        </section>

        <section v-if="activeTab === 'ledger'">
          <div class="toolbar"><div><h3>Usage archive</h3><p>Input, output, cache tokens and settlement formula.</p></div><button class="ghost" @click="loadLedger">Refresh</button></div>
          <div class="table-shell">
            <table><tbody><tr v-for="entry in ledger" :key="entry.id"><td>{{ entry.created_at }}</td><td>{{ entry.model }}</td><td>{{ entry.input_tokens }}/{{ entry.output_tokens }}/{{ entry.cache_tokens }}</td><td>{{ fmt(entry.total_points, 4) }}</td><td>{{ entry.tokenizer }}</td><td>{{ entry.formula_note }}</td></tr></tbody></table>
          </div>
        </section>

        <section v-if="activeTab === 'settings' && isAdmin">
          <div class="toolbar"><div><h3>Invite controls</h3><p>Local controls for invite-gated circles.</p></div><button @click="saveSettings">Save</button></div>
          <div class="form-grid compact panel">
            <label>Invite Required <select v-model="settingsForm.invite_required"><option value="false">false</option><option value="true">true</option></select></label>
            <label>Default Invite Code <input v-model="settingsForm.invite_code_default" /></label>
          </div>
          <div class="table-shell">
            <table><tbody><tr v-for="setting in settings" :key="setting.key"><td>{{ setting.key }}</td><td>{{ setting.value }}</td><td>{{ setting.updated_at }}</td></tr></tbody></table>
          </div>
        </section>
      </template>
    </section>
  </main>
</template>
