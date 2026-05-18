<script setup lang="ts">
import { computed, onMounted, reactive, ref } from 'vue'

type User = {
  id: number
  email: string
  role: string
  display_name: string
  points_balance: number
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

const token = ref(localStorage.getItem('tokenaltar_token') || '')
const user = ref<User | null>(null)
const error = ref('')
const activeTab = ref('dashboard')
const apiKeys = ref<any[]>([])
const channels = ref<any[]>([])
const prices = ref<any[]>([])
const rules = ref<any[]>([])
const ledger = ref<any[]>([])
const dashboard = ref<Dashboard | null>(null)
const newApiKey = ref('')

const loginForm = reactive({ email: 'admin@example.com', password: '' })
const apiKeyForm = reactive({ name: 'local-dev', spend_limit_points: 1000 })
const channelForm = reactive({
  name: 'OpenAI Pool',
  provider: 'openai',
  base_url: 'https://api.openai.com',
  api_key_secret: '',
  models: 'gpt-*',
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
  model_pattern: 'default',
  input_price_per_1k: 1,
  output_price_per_1k: 3,
  cache_price_per_1k: 0.2,
})
const ruleForm = reactive({
  name: 'tenant-session',
  enabled: true,
  model_regex: '.*',
  request_path: '/v1/responses',
  user_agent_regex: '',
  key_source_type: 'request_header',
  key_source_path: 'x-tenant-id',
  group_name: 'default',
  ttl_seconds: 3600,
  skip_retry_on_failure: false,
  switch_on_success: true,
})

const isAdmin = computed(() => user.value?.role === 'admin')
const tabs = computed(() => [
  ['dashboard', 'Dashboard'],
  ['keys', 'API Keys'],
  ['channels', 'Channels'],
  ['prices', 'Pricing'],
  ['affinity', 'Affinity'],
  ['ledger', 'Ledger'],
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
  if (!response.ok) {
    throw new Error(data?.error || response.statusText)
  }
  return data
}

async function login() {
  try {
    const data = await api('/auth/login', {
      method: 'POST',
      body: JSON.stringify(loginForm),
    })
    token.value = data.token
    user.value = data.user
    localStorage.setItem('tokenaltar_token', data.token)
    await refreshAll()
  } catch (err) {
    error.value = String(err)
  }
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
    ])
  } catch (err) {
    error.value = String(err)
    logout()
  }
}

async function loadDashboard() {
  dashboard.value = await api('/dashboard')
}

async function loadApiKeys() {
  apiKeys.value = await api('/api-keys')
}

async function createApiKey() {
  const data = await api('/api-keys', {
    method: 'POST',
    body: JSON.stringify(apiKeyForm),
  })
  newApiKey.value = data.token
  await loadApiKeys()
}

async function toggleApiKey(record: any) {
  await api(`/api-keys/${record.id}/enabled`, {
    method: 'POST',
    body: JSON.stringify({ enabled: !record.enabled }),
  })
  await loadApiKeys()
}

async function loadChannels() {
  channels.value = await api('/channels')
}

async function createChannel() {
  await api('/channels', {
    method: 'POST',
    body: JSON.stringify({
      ...channelForm,
      models: channelForm.models.split(',').map((item) => item.trim()).filter(Boolean),
    }),
  })
  channelForm.api_key_secret = ''
  await loadChannels()
  await loadDashboard()
}

async function loadPrices() {
  prices.value = await api('/prices')
}

async function savePrice() {
  await api('/prices', {
    method: 'POST',
    body: JSON.stringify(priceForm),
  })
  await loadPrices()
}

async function loadRules() {
  rules.value = await api('/affinity-rules')
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

async function loadLedger() {
  ledger.value = await api('/ledger')
}

onMounted(refreshAll)
</script>

<template>
  <main class="shell">
    <aside class="sidebar">
      <div class="brand">
        <div class="mark">TA</div>
        <div>
          <h1>TokenAltar</h1>
          <p>LLM capacity exchange</p>
        </div>
      </div>
      <nav v-if="user" class="tabs">
        <button
          v-for="[id, label] in tabs"
          :key="id"
          :class="{ active: activeTab === id }"
          @click="activeTab = id"
        >
          {{ label }}
        </button>
      </nav>
      <div v-if="user" class="account">
        <strong>{{ user.display_name }}</strong>
        <span>{{ user.email }}</span>
        <span>{{ user.points_balance.toFixed(2) }} points</span>
        <button class="ghost" @click="logout">Sign out</button>
      </div>
    </aside>

    <section class="content">
      <div v-if="error" class="error">{{ error }}</div>

      <section v-if="!user" class="login-panel">
        <h2>Console Login</h2>
        <label>Email <input v-model="loginForm.email" autocomplete="username" /></label>
        <label>Password <input v-model="loginForm.password" type="password" autocomplete="current-password" /></label>
        <button @click="login">Sign in</button>
      </section>

      <template v-else>
        <section v-if="activeTab === 'dashboard'">
          <div class="toolbar">
            <h2>Dashboard</h2>
            <button class="ghost" @click="refreshAll">Refresh</button>
          </div>
          <div class="metric-grid">
            <article><span>Surge</span><strong>{{ dashboard?.surge_state }}</strong><em>{{ dashboard?.surge_multiplier }}x</em></article>
            <article><span>Available Tokens</span><strong>{{ dashboard?.available_tokens.toLocaleString() }}</strong></article>
            <article><span>Channels</span><strong>{{ dashboard?.enabled_channels }} / {{ dashboard?.channels }}</strong></article>
            <article><span>Today Spend</span><strong>{{ dashboard?.spent_points_today.toFixed(4) }}</strong></article>
          </div>
        </section>

        <section v-if="activeTab === 'keys'">
          <div class="toolbar"><h2>API Keys</h2><button @click="createApiKey">Create</button></div>
          <div class="form-row">
            <label>Name <input v-model="apiKeyForm.name" /></label>
            <label>Spend Limit <input v-model.number="apiKeyForm.spend_limit_points" type="number" /></label>
          </div>
          <p v-if="newApiKey" class="secret">{{ newApiKey }}</p>
          <table><tbody><tr v-for="key in apiKeys" :key="key.id"><td>{{ key.name }}</td><td>{{ key.key_prefix }}</td><td>{{ key.spent_points.toFixed(4) }}</td><td><button class="ghost" @click="toggleApiKey(key)">{{ key.enabled ? 'Disable' : 'Enable' }}</button></td></tr></tbody></table>
        </section>

        <section v-if="activeTab === 'channels'">
          <div class="toolbar"><h2>Channels</h2><button :disabled="!isAdmin" @click="createChannel">Add Channel</button></div>
          <div class="form-grid">
            <label>Name <input v-model="channelForm.name" /></label>
            <label>Provider <select v-model="channelForm.provider"><option value="openai">OpenAI</option><option value="anthropic">Anthropic</option></select></label>
            <label>Base URL <input v-model="channelForm.base_url" /></label>
            <label>API Key <input v-model="channelForm.api_key_secret" type="password" /></label>
            <label>Models <input v-model="channelForm.models" /></label>
            <label>Cycle Limit <input v-model.number="channelForm.cycle_limit_tokens" type="number" /></label>
            <label>Daily Limit <input v-model.number="channelForm.daily_limit_tokens" type="number" /></label>
            <label>Hourly Limit <input v-model.number="channelForm.hourly_limit_tokens" type="number" /></label>
            <label>Fire Sale Discount <input v-model.number="channelForm.fire_sale_discount" type="number" step="0.01" /></label>
            <label>Provider Share <input v-model.number="channelForm.provider_share" type="number" step="0.01" /></label>
          </div>
          <table><tbody><tr v-for="channel in channels" :key="channel.id"><td>{{ channel.name }}</td><td>{{ channel.provider }}</td><td>{{ channel.status }}</td><td>{{ (channel.limits.cycle_limit_tokens - channel.limits.used_cycle_tokens).toLocaleString() }}</td></tr></tbody></table>
        </section>

        <section v-if="activeTab === 'prices'">
          <div class="toolbar"><h2>Pricing</h2><button :disabled="!isAdmin" @click="savePrice">Save</button></div>
          <div class="form-grid compact">
            <label>Model Pattern <input v-model="priceForm.model_pattern" /></label>
            <label>Input / 1k <input v-model.number="priceForm.input_price_per_1k" type="number" step="0.01" /></label>
            <label>Output / 1k <input v-model.number="priceForm.output_price_per_1k" type="number" step="0.01" /></label>
            <label>Cache / 1k <input v-model.number="priceForm.cache_price_per_1k" type="number" step="0.01" /></label>
          </div>
          <table><tbody><tr v-for="price in prices" :key="price.model_pattern"><td>{{ price.model_pattern }}</td><td>{{ price.input_price_per_1k }}</td><td>{{ price.output_price_per_1k }}</td><td>{{ price.cache_price_per_1k }}</td></tr></tbody></table>
        </section>

        <section v-if="activeTab === 'affinity'">
          <div class="toolbar"><h2>Affinity Rules</h2><button :disabled="!isAdmin" @click="createRule">Create</button></div>
          <div class="form-grid compact">
            <label>Name <input v-model="ruleForm.name" /></label>
            <label>Path <input v-model="ruleForm.request_path" /></label>
            <label>Model Regex <input v-model="ruleForm.model_regex" /></label>
            <label>Source <select v-model="ruleForm.key_source_type"><option value="request_header">Header</option><option value="json_path">JSON Path</option><option value="context">Context</option></select></label>
            <label>Source Path <input v-model="ruleForm.key_source_path" /></label>
            <label>TTL <input v-model.number="ruleForm.ttl_seconds" type="number" /></label>
          </div>
          <table><tbody><tr v-for="rule in rules" :key="rule.id"><td>{{ rule.name }}</td><td>{{ rule.request_path }}</td><td>{{ rule.key_source_type }}:{{ rule.key_source_path }}</td><td>{{ rule.ttl_seconds }}s</td></tr></tbody></table>
        </section>

        <section v-if="activeTab === 'ledger'">
          <div class="toolbar"><h2>Ledger</h2><button class="ghost" @click="loadLedger">Refresh</button></div>
          <table><tbody><tr v-for="entry in ledger" :key="entry.id"><td>{{ entry.created_at }}</td><td>{{ entry.model }}</td><td>{{ entry.input_tokens }}/{{ entry.output_tokens }}/{{ entry.cache_tokens }}</td><td>{{ entry.total_points.toFixed(4) }}</td><td>{{ entry.formula_note }}</td></tr></tbody></table>
        </section>
      </template>
    </section>
  </main>
</template>
