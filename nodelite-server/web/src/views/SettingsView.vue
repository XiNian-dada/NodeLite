<script setup lang="ts">
import { computed, onMounted } from 'vue';
import { useI18n } from 'vue-i18n';
import AppLayout from '@/components/AppLayout.vue';
import ServerUpdateCard from '@/components/ServerUpdateCard.vue';
import OpsCard from '@/components/OpsCard.vue';
import TokenTable from '@/components/TokenTable.vue';
import SettingsMessage from '@/components/SettingsMessage.vue';
import { tokenSeverity } from '@/lib/format';
import { useSettingsStore } from '@/stores/settings';

const { t } = useI18n();
const store = useSettingsStore();

onMounted(() => {
  void store.load();
});

const summaryTiles = computed(() => {
  const settings = store.data;
  if (!settings) return [];
  const agents = settings.agents;
  const online = agents.filter((agent) => agent.online).length;
  const offline = agents.length - online;
  const expired = agents.filter((agent) => tokenSeverity(agent.token_expires_in_secs) === 'expired').length;
  const expiring = agents.filter((agent) => tokenSeverity(agent.token_expires_in_secs) === 'expiring').length;
  const tokenHealth =
    expired > 0
      ? t('settings.summary.token_attention', { count: expired })
      : expiring > 0
        ? t('settings.summary.token_expiring', { count: expiring })
        : t('settings.summary.token_good');

  return [
    {
      key: 'version',
      label: t('settings.summary.version'),
      value: settings.server_version,
      tone: 'neutral',
    },
    {
      key: 'registered',
      label: t('settings.summary.registered'),
      value: String(agents.length),
      tone: 'neutral',
    },
    {
      key: 'online',
      label: t('common.online'),
      value: String(online),
      tone: 'green',
    },
    {
      key: 'offline',
      label: t('common.offline'),
      value: String(offline),
      tone: offline > 0 ? 'purple' : 'neutral',
    },
    {
      key: 'token',
      label: t('settings.summary.token_health'),
      value: tokenHealth,
      tone: expired > 0 ? 'red' : expiring > 0 ? 'yellow' : 'green',
    },
  ];
});
</script>

<template>
  <AppLayout>
    <template #title>
      <h1 class="page-heading">{{ t('settings.heading') }}</h1>
      <p class="page-subtitle">{{ t('settings.subtitle') }}</p>
    </template>

    <section class="settings" data-test="settings-view">
      <template v-if="store.data">
        <div class="settings-summary" data-test="settings-summary">
          <article
            v-for="tile in summaryTiles"
            :key="tile.key"
            class="summary-tile"
            :class="`summary-tile--${tile.tone}`"
          >
            <span>{{ tile.label }}</span>
            <strong>{{ tile.value }}</strong>
          </article>
        </div>

        <div class="settings__grid">
          <ServerUpdateCard class="settings__card" :settings="store.data" />
          <OpsCard class="settings__card" :settings="store.data" />
        </div>
        <TokenTable class="settings__tokens" :agents="store.data.agents" @saved="store.refresh()" />
      </template>
      <SettingsMessage
        v-else-if="store.error"
        state="error"
        :text="store.error.message"
        data-test="settings-error"
      />
      <p v-else class="placeholder" data-test="settings-loading">
        {{ t('common.waiting_for_data') }}
      </p>
    </section>
  </AppLayout>
</template>

<style scoped>
.settings {
  display: flex;
  flex-direction: column;
  gap: 16px;
  width: 100%;
}
.settings-summary {
  display: grid;
  grid-template-columns: repeat(5, minmax(0, 1fr));
  border: 1px solid var(--border-soft);
  border-radius: 8px;
  background: var(--bg-card);
  overflow: hidden;
}
.summary-tile {
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 16px 18px;
  border-right: 1px solid var(--border-soft);
}
.summary-tile:last-child {
  border-right: 0;
}
.summary-tile span {
  color: var(--text-muted);
  font-size: 12px;
}
.summary-tile strong {
  color: var(--text-primary);
  font-size: 22px;
  font-weight: 600;
  line-height: 1;
  font-variant-numeric: tabular-nums;
  overflow-wrap: anywhere;
}
.summary-tile::before {
  content: '';
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: currentColor;
  flex: 0 0 auto;
}
.summary-tile--neutral {
  color: var(--text-secondary);
}
.summary-tile--green {
  color: var(--accent-green);
}
.summary-tile--yellow {
  color: var(--accent-yellow);
}
.summary-tile--red {
  color: var(--accent-red);
}
.summary-tile--purple {
  color: var(--chart-network-up);
}
.settings__grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 16px;
  align-items: stretch;
}
.settings__card,
.settings__tokens {
  min-width: 0;
}
.settings__card {
  height: 100%;
}
.page-heading {
  margin: 0;
  font-size: 24px;
  font-weight: 600;
  letter-spacing: 0;
}
.page-subtitle {
  margin: 4px 0 0;
  color: var(--text-muted);
  font-size: 13px;
}
.placeholder {
  color: var(--text-muted);
  font-size: 13px;
}
@media (max-width: 880px) {
  .settings-summary {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
  .summary-tile {
    border-right: 0;
    border-bottom: 1px solid var(--border-soft);
  }
  .summary-tile:nth-child(odd) {
    border-right: 1px solid var(--border-soft);
  }
  .settings__grid {
    grid-template-columns: minmax(0, 1fr);
  }
}
@media (max-width: 520px) {
  .settings-summary {
    grid-template-columns: minmax(0, 1fr);
  }
  .summary-tile:nth-child(odd) {
    border-right: 0;
  }
}
</style>
