<script setup lang="ts">
import { providerTone } from '../utils/health'

defineProps<{
  provider: string
  compact?: boolean
}>()
</script>

<template>
  <span class="provider-badge" :class="[providerTone(provider), { compact }]">{{ provider }}</span>
</template>
