<script setup lang="ts">
import { onMounted, ref } from 'vue';

interface BootstrapResponse {
  service: string;
  status: string;
  ready: boolean;
  history_available: boolean;
  public_base_url: string | null;
  refresh_interval_secs: number;
  registered_nodes: number;
}

type LoadState =
  | { kind: 'idle' }
  | { kind: 'loading' }
  | { kind: 'ok'; data: BootstrapResponse }
  | { kind: 'error'; message: string };

const state = ref<LoadState>({ kind: 'idle' });

async function loadBootstrap(): Promise<void> {
  state.value = { kind: 'loading' };
  try {
    const res = await fetch('/api/bootstrap', {
      credentials: 'same-origin',
      headers: { Accept: 'application/json' },
    });
    if (!res.ok) {
      state.value = { kind: 'error', message: `HTTP ${res.status}` };
      return;
    }
    const data = (await res.json()) as BootstrapResponse;
    state.value = { kind: 'ok', data };
  } catch (error) {
    state.value = {
      kind: 'error',
      message: error instanceof Error ? error.message : String(error),
    };
  }
}

onMounted(loadBootstrap);
</script>

<template>
  <main class="hello">
    <h1>NodeLite — Vue scaffold</h1>
    <p class="hint">Stage 0 hello world. Verifies /api/bootstrap is reachable through the Vite dev proxy.</p>

    <section v-if="state.kind === 'loading'" data-test="bootstrap-loading">Loading…</section>

    <section v-else-if="state.kind === 'error'" data-test="bootstrap-error" class="error">
      Failed to load bootstrap: {{ state.message }}
    </section>

    <section v-else-if="state.kind === 'ok'" data-test="bootstrap-ok">
      <dl>
        <dt>service</dt>
        <dd>{{ state.data.service }}</dd>
        <dt>status</dt>
        <dd>{{ state.data.status }}</dd>
        <dt>ready</dt>
        <dd>{{ state.data.ready }}</dd>
        <dt>refresh_interval_secs</dt>
        <dd>{{ state.data.refresh_interval_secs }}</dd>
        <dt>registered_nodes</dt>
        <dd>{{ state.data.registered_nodes }}</dd>
      </dl>
    </section>

    <button type="button" @click="loadBootstrap">Reload</button>
  </main>
</template>

<style scoped>
.hello {
  font-family: ui-sans-serif, system-ui, -apple-system, sans-serif;
  padding: 2rem;
  max-width: 640px;
  margin: 0 auto;
  color: #e6edf3;
  background: #0e1422;
  min-height: 100vh;
}
h1 {
  margin: 0 0 0.25rem;
  font-size: 1.5rem;
}
.hint {
  color: #b6bfcc;
  font-size: 0.9rem;
}
.error {
  color: #ef4444;
}
dl {
  display: grid;
  grid-template-columns: 12rem 1fr;
  gap: 0.25rem 1rem;
  margin: 1rem 0;
}
dt {
  color: #6b7785;
  font-variant: small-caps;
}
dd {
  margin: 0;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
}
button {
  background: #3b82f6;
  color: #fff;
  border: 0;
  padding: 0.4rem 0.9rem;
  border-radius: 4px;
  cursor: pointer;
}
button:hover {
  background: #2563eb;
}
</style>
