<script setup lang="ts">
defineProps<{ state: 'ok' | 'error' | null; text: string }>();
</script>

<template>
  <p
    v-if="state && text"
    class="settings-message"
    :class="state"
    data-test="settings-message"
    role="status"
  >
    {{ text }}
  </p>
</template>

<style scoped>
.settings-message {
  margin: 8px 0 0;
  font-size: 13px;
}
.settings-message.ok {
  color: var(--accent-green);
}
.settings-message.error {
  color: var(--accent-red);
}
</style>
