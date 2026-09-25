<script setup lang="ts">
import { computed, onMounted, onUnmounted, reactive, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { blankRule, type RuleDraft } from '@/lib/alertsDraft';
import { ALERT_RULE_PRESETS, createRuleFromPreset } from '@/lib/alertsPresets';
import RuleEditorCard from './RuleEditorCard.vue';

/**
 * The rule collection editor. Binds the parent's reactive `draft.rules` array
 * and mutates it in place (push/splice) — the reactive draft is the single
 * source of truth, so add/remove never loses sibling edits. Each card is keyed
 * by the stable `uid` (not the array index, and not the user-editable `id`) so
 * Vue keeps each editor's local DOM state when rules are added or removed.
 */
const rules = defineModel<RuleDraft[]>({ required: true });

const { t } = useI18n();

// Track expansion state per rule by uid (defaults to true)
const openMap = reactive<Record<string, boolean>>({});
const showPresetMenu = ref(false);
const dropdownWrapRef = ref<HTMLElement | null>(null);

function isRuleOpen(uid: string): boolean {
  return openMap[uid] ?? true;
}

function setRuleOpen(uid: string, open: boolean): void {
  openMap[uid] = open;
}

const allOpen = computed(() => {
  if (!rules.value.length) return true;
  return rules.value.every((r) => isRuleOpen(r.uid));
});

function toggleAll(): void {
  const next = !allOpen.value;
  for (const r of rules.value) {
    openMap[r.uid] = next;
  }
}

function add(): void {
  const next = blankRule();
  openMap[next.uid] = true;
  rules.value.push(next);
}

function addPreset(presetKey: string): void {
  const next = createRuleFromPreset(presetKey, (key) => t(key));
  openMap[next.uid] = true;
  rules.value.push(next);
  showPresetMenu.value = false;
}

function togglePresetMenu(): void {
  showPresetMenu.value = !showPresetMenu.value;
}

function handleClickOutside(event: MouseEvent): void {
  if (dropdownWrapRef.value && !dropdownWrapRef.value.contains(event.target as Node)) {
    showPresetMenu.value = false;
  }
}

onMounted(() => {
  document.addEventListener('click', handleClickOutside);
});

onUnmounted(() => {
  document.removeEventListener('click', handleClickOutside);
});

function remove(index: number): void {
  const rule = rules.value[index];
  if (rule) delete openMap[rule.uid];
  rules.value.splice(index, 1);
}

// Fired only if a card reassigns its whole model; field edits mutate in place.
function update(index: number, next: RuleDraft): void {
  rules.value[index] = next;
}
</script>

<template>
  <article class="panel rules" data-test="rule-list">
    <header class="rules-head">
      <div class="rules-intro">
        <h2 class="card-title">{{ t('alerts.rules.title') }}</h2>
        <p class="rules-note">{{ t('alerts.rules.note') }}</p>
      </div>
      <div class="rules-actions">
        <button
          v-if="rules.length > 0"
          type="button"
          class="btn btn--subtle btn--sm"
          data-test="rule-toggle-all"
          @click="toggleAll"
        >
          {{ allOpen ? t('alerts.rules.collapse_all') : t('alerts.rules.expand_all') }}
        </button>

        <div ref="dropdownWrapRef" class="preset-dropdown-wrap">
          <button
            type="button"
            class="btn btn--subtle btn--sm"
            data-test="rule-add-preset-btn"
            @click="togglePresetMenu"
          >
            {{ t('alerts.rules.presets.add_from_preset') }} ▾
          </button>
          <div v-if="showPresetMenu" class="preset-menu" data-test="rule-preset-menu">
            <button
              v-for="preset in ALERT_RULE_PRESETS"
              :key="preset.key"
              type="button"
              class="preset-menu-item"
              :data-test="`preset-menu-item-${preset.key}`"
              @click="addPreset(preset.key)"
            >
              <span class="preset-menu-icon">{{ preset.icon }}</span>
              <div class="preset-menu-content">
                <span class="preset-menu-title">{{ t(preset.nameKey) }}</span>
                <span class="preset-menu-desc">{{ t(preset.descKey) }}</span>
              </div>
            </button>
          </div>
        </div>

        <button type="button" class="btn btn--primary btn--sm" data-test="rule-add" @click="add">
          {{ t('alerts.rules.add') }}
        </button>
      </div>
    </header>

    <div v-if="!rules.length" class="rules-empty-wrap" data-test="rule-list-empty">
      <div class="preset-section">
        <div class="preset-header">
          <h3 class="preset-title">{{ t('alerts.rules.presets.title') }}</h3>
          <p class="preset-subtitle">{{ t('alerts.rules.presets.subtitle') }}</p>
        </div>
        <div class="preset-grid">
          <div
            v-for="preset in ALERT_RULE_PRESETS"
            :key="preset.key"
            class="preset-card"
            :data-test="`preset-card-${preset.key}`"
            role="button"
            tabindex="0"
            @click="addPreset(preset.key)"
            @keydown.enter.prevent="addPreset(preset.key)"
            @keydown.space.prevent="addPreset(preset.key)"
          >
            <div class="preset-card-top">
              <span class="preset-card-icon">{{ preset.icon }}</span>
              <span class="preset-card-name">{{ t(preset.nameKey) }}</span>
            </div>
            <p class="preset-card-desc">{{ t(preset.descKey) }}</p>
            <button
              type="button"
              class="btn btn--subtle btn--sm preset-card-btn"
              :data-test="`preset-add-${preset.key}`"
              @click.stop="addPreset(preset.key)"
            >
              + {{ t('alerts.rules.presets.add') }}
            </button>
          </div>
        </div>
      </div>
    </div>
    <div v-else class="rules-items">
      <RuleEditorCard
        v-for="(rule, index) in rules"
        :key="rule.uid"
        :model-value="rule"
        :open="isRuleOpen(rule.uid)"
        @update:model-value="(next) => update(index, next)"
        @update:open="(val) => setRuleOpen(rule.uid, val)"
        @remove="remove(index)"
      />
    </div>
  </article>
</template>

<style scoped>
.panel {
  background: var(--bg-card);
  border: 1px solid var(--border-soft);
  border-radius: 8px;
  padding: 16px;
}
.rules-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}
.rules-intro {
  min-width: 0;
}
.rules-actions {
  display: flex;
  align-items: center;
  gap: var(--actions-gap);
  flex-shrink: 0;
  position: relative;
}
.card-title {
  margin: 0;
  font-size: 16px;
  font-weight: 600;
}
.rules-note {
  margin: 4px 0 0;
  color: var(--text-muted);
  font-size: 12px;
}
.rules-items {
  display: flex;
  flex-direction: column;
  gap: 12px;
  margin-top: 14px;
}

.preset-dropdown-wrap {
  position: relative;
}

.preset-menu {
  position: absolute;
  top: calc(100% + 6px);
  right: 0;
  width: 260px;
  background: var(--bg-card-soft);
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-md);
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.35);
  padding: 6px;
  z-index: 50;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.preset-menu-item {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  padding: 8px 10px;
  border: none;
  background: transparent;
  border-radius: var(--radius-sm);
  cursor: pointer;
  text-align: left;
  transition: background-color 0.15s ease;
  width: 100%;
}

.preset-menu-item:hover {
  background: var(--bg-elevated);
}

.preset-menu-icon {
  font-size: 16px;
  line-height: 1.2;
}

.preset-menu-content {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.preset-menu-title {
  font-size: 13px;
  font-weight: 500;
  color: var(--text-primary);
}

.preset-menu-desc {
  font-size: 11px;
  color: var(--text-muted);
  line-height: 1.3;
}

.rules-empty-wrap {
  margin-top: 16px;
}

.preset-section {
  padding: 16px;
  background: var(--bg-card-soft);
  border: 1px dashed var(--border-soft);
  border-radius: var(--radius-md);
}

.preset-header {
  margin-bottom: 12px;
}

.preset-title {
  margin: 0;
  font-size: 14px;
  font-weight: 600;
  color: var(--text-primary);
}

.preset-subtitle {
  margin: 4px 0 0;
  font-size: 12px;
  color: var(--text-muted);
}

.preset-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
  gap: 10px;
}

.preset-card {
  display: flex;
  flex-direction: column;
  justify-content: space-between;
  padding: 12px;
  background: var(--bg-card);
  border: 1px solid var(--border-soft);
  border-radius: var(--radius-sm);
  cursor: pointer;
  transition: all 0.15s ease;
}

.preset-card:hover {
  border-color: var(--border-strong);
  background: var(--bg-elevated);
}

.preset-card-top {
  display: flex;
  align-items: center;
  gap: 8px;
}

.preset-card-icon {
  font-size: 18px;
}

.preset-card-name {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-primary);
}

.preset-card-desc {
  margin: 8px 0 12px;
  font-size: 11px;
  color: var(--text-muted);
  line-height: 1.4;
  flex: 1;
}

.preset-card-btn {
  width: 100%;
  justify-content: center;
  font-size: 12px;
}
</style>
