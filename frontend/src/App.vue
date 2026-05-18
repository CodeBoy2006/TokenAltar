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
const settings = ref<any[]>([])
const dashboard = ref<Dashboard | null>(null)
const newApiKey = ref('')
const claimResult = ref('')

const loginForm = reactive({ email: 'admin@example.com', password: '' })
const registerForm = reactive({ email: '', password: '', display_name: '', invite_code: '' })
const apiKeyForm = reactive({ name: 'local-dev', spend_limit_points: 1000 })
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
async function loadLeaderboards() { leaderboards.value = await api('/leaderboards') }

async function loadSettings() {
  settings.value = await api('/settings')
  for (const setting of settings.value) {
    if (setting.key in settingsForm) {
      ;(settingsForm as any)[setting.key] = setting.value
    }
  }
}

async function createApiKey() {
  const data = await api('/api-keys', { method: 'POST', body: JSON.stringify(apiKeyForm) })
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

async function createChannel() {
  await api('/channels', {
    method: 'POST',
    body: JSON.stringify({
      ...channelForm,
      models: channelForm.models.split(',').map((item) => item.trim()).filter(Boolean),
    }),
  })
  channelForm.api_key_secret = ''
  await Promise.all([loadChannels(), loadDashboard()])
  if (!isAdmin.value && !priceForm.channel_id && channels.value.length > 0) {
    priceForm.channel_id = channels.value[0].id
  }
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
          <div class="hero-copy">
            <span>Private token exchange</span>
            <h2>TokenAltar</h2>
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
          <div class="toolbar"><div><h3>Issue a key</h3><p>Per-key spend limits prevent runaway clients.</p></div><button @click="createApiKey">Create</button></div>
          <div class="form-row panel">
            <label>Name <input v-model="apiKeyForm.name" /></label>
            <label>Spend Limit <input v-model.number="apiKeyForm.spend_limit_points" type="number" /></label>
          </div>
          <p v-if="newApiKey" class="secret">{{ newApiKey }}</p>
          <div class="table-shell">
            <table><tbody><tr v-for="key in apiKeys" :key="key.id"><td>{{ key.name }}</td><td><code>{{ key.key_prefix }}</code></td><td>{{ fmt(key.spent_points, 4) }}</td><td><button class="ghost" @click="toggleApiKey(key)">{{ key.enabled ? 'Disable' : 'Enable' }}</button></td></tr></tbody></table>
          </div>
        </section>

        <section v-if="activeTab === 'channels'">
          <div class="toolbar"><div><h3>Add upstream capacity</h3><p>Monthly, daily, and hourly token buckets drive routing.</p></div><button @click="createChannel">Add Channel</button></div>
          <div class="form-grid panel">
            <label>Name <input v-model="channelForm.name" /></label>
            <label>Provider <select v-model="channelForm.provider"><option value="openai">OpenAI</option><option value="anthropic">Anthropic</option></select></label>
            <label>Base URL <input v-model="channelForm.base_url" /></label>
            <label>API Key <input v-model="channelForm.api_key_secret" type="password" /></label>
            <label>Models <input v-model="channelForm.models" /></label>
            <label>Reset Day <input v-model.number="channelForm.cycle_reset_day" type="number" min="1" max="28" /></label>
            <label>Cycle Limit <input v-model.number="channelForm.cycle_limit_tokens" type="number" /></label>
            <label>Daily Limit <input v-model.number="channelForm.daily_limit_tokens" type="number" /></label>
            <label>Hourly Limit <input v-model.number="channelForm.hourly_limit_tokens" type="number" /></label>
            <label>Fire Sale Days <input v-model.number="channelForm.fire_sale_days_before" type="number" /></label>
            <label>Fire Sale Discount <input v-model.number="channelForm.fire_sale_discount" type="number" step="0.01" /></label>
            <label>Provider Share <input v-model.number="channelForm.provider_share" type="number" step="0.01" /></label>
          </div>
          <div class="table-shell">
            <table><tbody><tr v-for="channel in channels" :key="channel.id"><td>{{ channel.name }}</td><td>{{ channel.provider }}</td><td><span class="status">{{ channel.status }}</span></td><td>{{ fmt(channel.limits.cycle_limit_tokens - channel.limits.used_cycle_tokens, 0) }}</td><td>{{ channel.models.join(', ') || '*' }}</td></tr></tbody></table>
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
          <div class="toolbar"><div><h3>Monthly honors</h3><p>Provider tokens and consumer point burn.</p></div><button class="ghost" @click="loadLeaderboards">Refresh</button></div>
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
