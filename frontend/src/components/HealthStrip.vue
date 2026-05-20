<script setup lang="ts">
import { computed } from 'vue'
import type { ChannelHealthWindow } from '../api/types'
import { healthBarClass, healthWindowTitle } from '../utils/health'

const props = withDefaults(defineProps<{
  windows: ChannelHealthWindow[]
  channelName: string
  variant?: 'regular' | 'large' | 'compact'
  limit?: number
  keyPrefix?: string
}>(), {
  variant: 'regular',
  limit: 0,
  keyPrefix: '',
})

const visibleWindows = computed(() => (
  props.limit > 0 ? props.windows.slice(-props.limit) : props.windows
))
const rootClass = computed(() => (
  props.variant === 'compact' ? 'mini-health-strip compact' : ['health-strip', { large: props.variant === 'large' }]
))
const label = computed(() => `Channel health windows for ${props.channelName}`)

function windowKey(window: ChannelHealthWindow) {
  return `${props.keyPrefix || props.channelName}:${window.window_start_at}`
}
</script>

<template>
  <div :class="rootClass" :aria-label="label">
    <span
      v-for="window in visibleWindows"
      :key="windowKey(window)"
      class="health-window"
      :class="healthBarClass(window)"
      :title="props.variant === 'compact' ? undefined : healthWindowTitle(window)"
    ></span>
  </div>
</template>
