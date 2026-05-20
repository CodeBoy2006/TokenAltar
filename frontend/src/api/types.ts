export type User = {
  id: number
  email: string
  role: string
  display_name: string
  points_balance: number
  anonymous_leaderboard: boolean
  enabled: boolean
}

export type ManagedUser = User & {
  disabled_at: string | null
  created_at: string
  updated_at: string
  api_key_count: number
  channel_count: number
  total_spent_points: number
  total_provider_points: number
}

export type Dashboard = {
  users: number
  channels: number
  enabled_channels: number
  available_tokens: number
  spent_points_today: number
  surge_multiplier: number
  surge_state: string
}

export type TabId =
  | 'dashboard'
  | 'users'
  | 'keys'
  | 'health'
  | 'channels'
  | 'prices'
  | 'affinity'
  | 'economy'
  | 'leaderboards'
  | 'ledger'
  | 'guide'
  | 'settings'

export type TabItem = [TabId, string]

export type ApiKeyRecord = {
  id: number
  user_id: number
  name: string
  key_prefix: string
  enabled: boolean
  spend_limit_points: number | null
  spent_points: number
  expires_at: string | null
  allowed_models: string[]
  allowed_channel_ids: number[]
  last_used_at: string | null
  status?: string
}

export type ChannelHealthStatus = 'available' | 'empty' | 'degraded' | 'down' | 'unknown'

export type ChannelHealthWindow = {
  window_start_at: string
  window_end_at: string
  status: ChannelHealthStatus
  sample_count: number
  success_count: number
  empty_count: number
  degraded_count: number
  down_count: number
  avg_ttft_ms: number | null
}

export type ChannelHealthSummary = {
  label: string
  detail: string
  tone: 'gray' | 'olive' | 'gold' | 'lapis' | 'wine'
}

export type ChannelQuotaWindow = {
  id?: number
  name: string
  limit_tokens: number
  used_tokens: number
  period_unit: string
  period_count: number
  anchor_at: string
  timezone: string
  current_window_start_at?: string
  current_window_end_at?: string
  sort_order?: number
}

export type ChannelLimits = {
  windows: ChannelQuotaWindow[]
  fire_sale_days_before: number
  fire_sale_remaining_pct: number
  fire_sale_discount: number
}

export type Channel = {
  id: number
  owner_user_id: number
  owner_display_name: string | null
  name: string
  provider: string
  base_url: string
  models: string[]
  enabled: boolean
  status: string
  health_checked_at: string | null
  upstream_latency_ms: number | null
  last_error: string | null
  limits: ChannelLimits
  health_windows: ChannelHealthWindow[]
}

export type DefaultChannelWindow = {
  name: string
  limit_tokens: number
  period_unit: string
  period_count: number
  timezone: string
}

export type EditableChannelWindow = DefaultChannelWindow & {
  anchor_at: string
}

export type ModelPrice = {
  channel_id: number | null
  model_pattern: string
  input_price_per_1m: number
  output_price_per_1m: number
  cache_price_per_1m: number
}

export type AffinityRule = {
  id: number
  name: string
  enabled: boolean
  model_regex: string | null
  request_path: string | null
  user_agent_regex: string | null
  key_source_type: string
  key_source_path: string
  group_name: string
  ttl_seconds: number
  skip_retry_on_failure: boolean
  switch_on_success: boolean
  include_model_name: boolean
}

export type LedgerEntry = {
  id: number
  created_at: string
  model: string
  input_tokens: number
  output_tokens: number
  cache_tokens: number
  total_points: number
  tokenizer: string
  formula_note: string
}

export type TransferRecord = {
  id: number
  from_user_id: number
  to_user_id: number
  from_name: string
  to_name: string
  points: number
  memo: string | null
  created_at: string
}

export type RedPacketRecord = {
  id: number
  phrase: string
  total_points: number
  remaining_points: number
  total_parts: number
  claimed_parts: number
  mode: string
  created_at: string
}

export type LeaderboardRow = {
  user_id: number | null
  name: string
  score: number
}

export type LeaderboardPayload = {
  period?: 'day' | 'month'
  timezone?: string
  window_start?: string
  providers: LeaderboardRow[]
  consumers: LeaderboardRow[]
}

export type RankedLeaderboardRow = LeaderboardRow & {
  key: string
  rank: number
  scoreText: string
  share: number
  tone: 'gold' | 'lapis' | 'olive' | 'plain'
}

export type ConsoleUpdateEvent = {
  id?: number
  topics?: string[]
}

export type RuntimeSettings = {
  [key: string]: unknown
  invite_required?: boolean
  invite_code_default?: string
  initial_admin_points?: number
  initial_user_points?: number
  pricing_unit_tokens?: number
  settlement_round_digits?: number
  default_api_key_spend_limit_points?: number
  default_channel_name?: string
  default_channel_provider?: string
  default_channel_base_url?: string
  default_channel_models?: string
  default_channel_windows?: DefaultChannelWindow[]
  default_channel_fire_sale_days_before?: number
  default_channel_fire_sale_remaining_pct?: number
  default_channel_fire_sale_discount?: number
  default_channel_provider_share?: number
  fallback_input_price_per_unit?: number
  fallback_output_price_per_unit?: number
  fallback_cache_price_per_unit?: number
}

export type SettingRecord = {
  key: string
  value: string
  updated_at: string
}

export type SettingsPayload = {
  settings: SettingRecord[]
  runtime: RuntimeSettings
}

export type AuthResponse = {
  token: string
  user: User
}

export type ApiKeyCreateResponse = {
  token: string
  record: ApiKeyRecord
}

export type ChannelTestResult = {
  ok: boolean
  latency_ms: number
  message: string
}
