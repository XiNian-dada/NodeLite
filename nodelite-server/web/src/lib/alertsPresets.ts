import { nextSeq, type RuleDraft } from './alertsDraft';

export interface AlertRulePreset {
  key: string;
  nameKey: string;
  descKey: string;
  icon: string;
  draft: Omit<RuleDraft, 'uid' | 'id' | 'name'> & { idPrefix: string };
}

export const ALERT_RULE_PRESETS: AlertRulePreset[] = [
  {
    key: 'offline',
    nameKey: 'alerts.rules.preset.offline.name',
    descKey: 'alerts.rules.preset.offline.desc',
    icon: '🔌',
    draft: {
      idPrefix: 'offline',
      enabled: true,
      metric: 'offline_minutes',
      comparator: 'gt',
      threshold: 5,
      window_minutes: 5,
      cooldown_minutes: 30,
      severity: 'critical',
      scope_mode: 'all',
      node_ids: [],
      tags: [],
      delivery: ['smtp', 'webhook'],
      send_resolved: true,
    },
  },
  {
    key: 'latency',
    nameKey: 'alerts.rules.preset.latency.name',
    descKey: 'alerts.rules.preset.latency.desc',
    icon: '⚡',
    draft: {
      idPrefix: 'latency',
      enabled: true,
      metric: 'latency_ms',
      comparator: 'gt',
      threshold: 250,
      window_minutes: 3,
      cooldown_minutes: 30,
      severity: 'warning',
      scope_mode: 'all',
      node_ids: [],
      tags: [],
      delivery: ['smtp', 'webhook'],
      send_resolved: true,
    },
  },
  {
    key: 'cpu',
    nameKey: 'alerts.rules.preset.cpu.name',
    descKey: 'alerts.rules.preset.cpu.desc',
    icon: '🔥',
    draft: {
      idPrefix: 'high-cpu',
      enabled: true,
      metric: 'cpu_usage_percent',
      comparator: 'gt',
      threshold: 90,
      window_minutes: 5,
      cooldown_minutes: 30,
      severity: 'warning',
      scope_mode: 'all',
      node_ids: [],
      tags: [],
      delivery: ['smtp', 'webhook'],
      send_resolved: true,
    },
  },
  {
    key: 'memory',
    nameKey: 'alerts.rules.preset.memory.name',
    descKey: 'alerts.rules.preset.memory.desc',
    icon: '🧠',
    draft: {
      idPrefix: 'high-memory',
      enabled: true,
      metric: 'memory_usage_percent',
      comparator: 'gt',
      threshold: 90,
      window_minutes: 5,
      cooldown_minutes: 30,
      severity: 'warning',
      scope_mode: 'all',
      node_ids: [],
      tags: [],
      delivery: ['smtp', 'webhook'],
      send_resolved: true,
    },
  },
  {
    key: 'disk',
    nameKey: 'alerts.rules.preset.disk.name',
    descKey: 'alerts.rules.preset.disk.desc',
    icon: '💾',
    draft: {
      idPrefix: 'high-disk',
      enabled: true,
      metric: 'disk_usage_percent',
      comparator: 'gt',
      threshold: 85,
      window_minutes: 10,
      cooldown_minutes: 60,
      severity: 'warning',
      scope_mode: 'all',
      node_ids: [],
      tags: [],
      delivery: ['smtp', 'webhook'],
      send_resolved: true,
    },
  },
];

export function createRuleFromPreset(
  presetKey: string,
  translateName?: (key: string) => string,
): RuleDraft {
  const preset = ALERT_RULE_PRESETS.find((p) => p.key === presetKey) || ALERT_RULE_PRESETS[0]!;
  const seq = nextSeq();
  return {
    ...preset.draft,
    uid: `rule-uid-${seq}`,
    id: `${preset.draft.idPrefix}-${seq}`,
    name: translateName ? translateName(preset.nameKey) : '',
    node_ids: [...preset.draft.node_ids],
    tags: [...preset.draft.tags],
    delivery: [...preset.draft.delivery],
  };
}
