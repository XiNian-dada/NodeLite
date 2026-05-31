import type {
  AgentLogEntry,
  AlertRuleView,
  AlertSettingsResponse,
  AlertSettingsView,
  AlertSmtpSettingsView,
  AlertWebhookSettingsView,
  AlertPreview,
  BootstrapResponse,
  InspectionSettingsView,
  NodeListItem,
  NodeStatus,
  OverviewData,
  SettingsResponse,
} from '@/api';

/** Override shape for the alert View fixture: nested channels accept partials. */
interface AlertSettingsViewOverrides {
  enabled?: boolean;
  smtp?: Partial<AlertSmtpSettingsView>;
  webhook?: Partial<AlertWebhookSettingsView>;
  rules?: AlertRuleView[];
  inspection?: Partial<InspectionSettingsView>;
}

export function makeSettings(overrides: Partial<SettingsResponse> = {}): SettingsResponse {
  const base: SettingsResponse = {
    service: 'nodelite-server',
    server_version: '2.3.0',
    repository: 'https://github.com/XiNian-dada/NodeLite',
    public_base_url: 'http://localhost:8080',
    listen: '127.0.0.1:8080',
    config_path: '/etc/nodelite/server.toml',
    registry_path: '/var/lib/nodelite/registry.json',
    history_db_path: '/var/lib/nodelite/history.db',
    snapshot_path: '/var/lib/nodelite/snapshot.json',
    history_retention_hours: 336,
    refresh_interval_secs: 5,
    auth: {
      enabled: true,
      username: 'admin',
      two_factor_enabled: false,
      totp_secret_configured: false,
      session_ttl_secs: 86_400,
      pending_ttl_secs: 300,
    },
    updates: {
      latest_release_url: 'https://github.com/XiNian-dada/NodeLite/releases/latest',
      server_upgrade_command: 'curl -fsSL https://example/install.sh | sh',
      agent_upgrade_command: 'curl -fsSL https://example/agent.sh | sh',
    },
    agents: [
      {
        node_id: 'node-a',
        node_label: 'Node A',
        online: true,
        agent_version: '2.3.0',
        remote_ip: '203.0.113.7',
        tags: [],
        token_expires_at: '2026-12-01T00:00:00Z',
        token_expires_in_secs: 1_000_000,
      },
    ],
  };
  return {
    ...base,
    ...overrides,
    auth: { ...base.auth, ...overrides.auth },
    updates: { ...base.updates, ...overrides.updates },
    agents: overrides.agents ?? base.agents,
  };
}

export function makeBootstrap(
  overrides: Partial<BootstrapResponse> = {},
): BootstrapResponse {
  return {
    service: 'nodelite-server',
    status: 'ready',
    ready: true,
    history_available: true,
    public_base_url: 'http://localhost:8080',
    refresh_interval_secs: 5,
    registered_nodes: 3,
    ...overrides,
  };
}

export function makeNode(overrides: Partial<NodeListItem> = {}): NodeListItem {
  // Honor explicitly-passed values by key presence so tests can set
  // latency_ms / snapshot to null (a plain ?? would coalesce null away).
  const defaultSnapshot = {
    cpu_usage_percent: 12.5,
    load: { one: 0.3 },
    memory: { total_bytes: 8_000_000_000, used_bytes: 2_000_000_000 },
  };
  return {
    identity: {
      node_id: 'node-a',
      node_label: 'Node A',
      hostname: 'host-a',
      tags: [],
      ...overrides.identity,
    },
    snapshot: 'snapshot' in overrides ? (overrides.snapshot ?? null) : defaultSnapshot,
    latency_ms: 'latency_ms' in overrides ? (overrides.latency_ms ?? null) : 5,
    online: 'online' in overrides ? (overrides.online ?? false) : true,
  };
}

export function makeNodeStatus(overrides: Partial<NodeStatus> = {}): NodeStatus {
  return {
    identity: {
      node_id: 'node-a',
      node_label: 'Node A',
      hostname: 'host-a',
      os: 'linux',
      kernel_version: '6.1.0',
      cpu_model: 'Test CPU',
      cpu_cores: 4,
      agent_version: '1.0.0',
      boot_time: '2026-05-28T00:00:00Z',
      tags: [],
      ...overrides.identity,
    },
    remote_ip: 'remote_ip' in overrides ? (overrides.remote_ip ?? null) : '203.0.113.7',
    snapshot:
      'snapshot' in overrides
        ? (overrides.snapshot ?? null)
        : {
            collected_at: '2026-05-29T00:00:00Z',
            cpu_usage_percent: 12.5,
            load: { one: 0.3, five: 0.4, fifteen: 0.5 },
            memory: {
              total_bytes: 8_000_000_000,
              used_bytes: 2_000_000_000,
              available_bytes: 6_000_000_000,
              swap_total_bytes: 0,
              swap_used_bytes: 0,
            },
            uptime_secs: 90_000,
            disks: [
              {
                device: '/dev/sda1',
                mount_point: '/',
                fs_type: 'ext4',
                total_bytes: 100_000_000_000,
                available_bytes: 60_000_000_000,
                used_bytes: 40_000_000_000,
                used_percent: 40,
              },
            ],
            network: {
              total_rx_bytes: 1000,
              total_tx_bytes: 2000,
              rx_bytes_per_sec: 10,
              tx_bytes_per_sec: 20,
            },
          },
    last_seen: 'last_seen' in overrides ? (overrides.last_seen ?? null) : '2026-05-29T00:00:00Z',
    latency_ms: 'latency_ms' in overrides ? (overrides.latency_ms ?? null) : 5,
    online: 'online' in overrides ? (overrides.online ?? false) : true,
  };
}

export function makeLogEntry(overrides: Partial<AgentLogEntry> = {}): AgentLogEntry {
  return {
    occurred_at: '2026-05-29T00:00:00Z',
    level: 'info',
    message: 'hello',
    ...overrides,
  };
}

export function makeOverview(overrides: Partial<OverviewData> = {}): OverviewData {
  return {
    generated_at: '2026-05-29T00:00:00Z',
    total_nodes: 3,
    online_nodes: 2,
    offline_nodes: 1,
    total_rx_bytes: 1000,
    total_tx_bytes: 2000,
    current_rx_bytes_per_sec: 10,
    current_tx_bytes_per_sec: 20,
    average_latency_ms: 7.5,
    ...overrides,
  };
}

export function makeAlertSettingsView(overrides: AlertSettingsViewOverrides = {}): AlertSettingsView {
  const base: AlertSettingsView = {
    enabled: true,
    smtp: {
      enabled: true,
      host: 'smtp.example.com',
      port: 587,
      username: 'mailer',
      sender: 'alerts@example.com',
      recipients: ['ops@example.com'],
      transport: 'start_tls',
      send_resolved: true,
      password_configured: true,
    },
    webhook: {
      enabled: false,
      url: '',
      send_resolved: true,
      secret_configured: false,
    },
    rules: [
      {
        id: 'cpu-hot',
        name: 'CPU hot',
        enabled: true,
        metric: 'cpu_usage_percent',
        comparator: 'gt',
        threshold: 85,
        window_minutes: 5,
        severity: 'warning',
        scope_mode: 'all',
        node_ids: [],
        tags: [],
        delivery: ['smtp'],
        cooldown_minutes: 30,
        send_resolved: true,
      },
    ],
    inspection: {
      enabled: true,
      local_time: '09:00',
      lookback_hours: 24,
      delivery: ['smtp'],
      offline_grace_minutes: 10,
      latency_warn_ms: 250,
      cpu_warn_percent: 85,
      memory_warn_percent: 90,
    },
  };
  return {
    ...base,
    ...overrides,
    smtp: { ...base.smtp, ...overrides.smtp },
    webhook: { ...base.webhook, ...overrides.webhook },
    inspection: { ...base.inspection, ...overrides.inspection },
    rules: overrides.rules ?? base.rules,
  };
}

export function makeAlertPreview(overrides: Partial<AlertPreview> = {}): AlertPreview {
  return {
    generated_at: '2026-05-29T00:00:00Z',
    triggered_rules: [],
    inspection: {
      total_nodes: 3,
      offline_nodes: 0,
      latency_nodes: 0,
      cpu_hot_nodes: 0,
      memory_hot_nodes: 0,
      highlights: [],
    },
    ...overrides,
  };
}

interface AlertSettingsResponseOverrides {
  config?: AlertSettingsViewOverrides;
  preview?: Partial<AlertPreview>;
}

export function makeAlertSettings(
  overrides: AlertSettingsResponseOverrides = {},
): AlertSettingsResponse {
  return {
    config: makeAlertSettingsView(overrides.config),
    preview: makeAlertPreview(overrides.preview),
  };
}
