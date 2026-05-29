<script setup lang="ts">
// App is a thin shell now — layout + data wiring live in the route views.
// Theme/24h bootstrap runs from the inline shim in index.html before this
// module loads (see src/composables/useTheme.ts, src/auth/expiry.ts).
</script>

<template>
  <RouterView />
</template>
