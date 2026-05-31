/**
 * Pure bridge between the server's alert View shapes and the editable draft the
 * AlertsView binds with v-model. Replaces the legacy DOM-as-state model
 * (assets/index-alert-settings.js `syncAlertDraftFromDom`): here the reactive
 * draft is the single source of truth, and these helpers only translate at the
 * two boundaries — load (View → draft) and save (draft → request payload).
 *
 * Secret handling mirrors the server merge rule (handlers/settings/alerts.rs):
 * `clear_*` wipes; else a non-blank typed value replaces; else the stored secret
 * is kept. Stored secrets are never echoed into the draft — `*_configured` is the
 * only signal the UI gets, and it drives a "leave blank to keep" placeholder.
 */

import type {
  AlertRuleView,
  AlertSettingsView,
  UpdateAlertRuleRequest,
  UpdateAlertSettingsRequest,
  UpdateAlertSmtpSettingsRequest,
  UpdateAlertWebhookSettingsRequest,
} from '@/api';

export interface SmtpDraft {
  enabled: boolean;
  host: string;
  port: number;
  username: string;
  sender: string;
  recipients: string[];
  transport: AlertSettingsView['smtp']['transport'];
  send_resolved: boolean;
  /** Read-only from the server: a secret is on file. The secret itself is never sent down. */
  password_configured: boolean;
  /** Transient input: empty = nothing typed (keep stored secret unless cleared). */
  password: string;
  /** Transient input: wipe the stored secret. */
  clear_password: boolean;
}

export interface WebhookDraft {
  enabled: boolean;
  url: string;
  send_resolved: boolean;
  secret_configured: boolean;
  secret: string;
  clear_secret: boolean;
}

/** A rule plus a stable client-side key for v-for (the server `id` is user-editable). */
export interface RuleDraft extends AlertRuleView {
  uid: string;
}

export interface AlertsDraft {
  enabled: boolean;
  smtp: SmtpDraft;
  webhook: WebhookDraft;
  rules: RuleDraft[];
  inspection: AlertSettingsView['inspection'];
}

/** Reauth carried by the save; matches the server's per-handler confirmation check. */
export interface ReauthInput {
  current_password: string;
  code: string;
}

let ruleSeq = 0;
function nextSeq(): number {
  ruleSeq += 1;
  return ruleSeq;
}

/** Defaults matching the legacy `emptyAlertsConfig` (used before the first load). */
export function emptyAlertsConfig(): AlertsDraft {
  return {
    enabled: false,
    smtp: {
      enabled: false,
      host: '',
      port: 587,
      username: '',
      sender: '',
      recipients: [],
      transport: 'start_tls',
      send_resolved: true,
      password_configured: false,
      password: '',
      clear_password: false,
    },
    webhook: {
      enabled: false,
      url: '',
      send_resolved: true,
      secret_configured: false,
      secret: '',
      clear_secret: false,
    },
    rules: [],
    inspection: {
      enabled: false,
      local_time: '09:00',
      lookback_hours: 24,
      delivery: ['smtp'],
      offline_grace_minutes: 10,
      latency_warn_ms: 250,
      cpu_warn_percent: 85,
      memory_warn_percent: 90,
    },
  };
}

/** A fresh rule with sensible defaults and a unique uid/id (for "add rule"). */
export function blankRule(): RuleDraft {
  const seq = nextSeq();
  return {
    uid: `rule-uid-${seq}`,
    id: `rule-${seq}`,
    name: '',
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
  };
}

function ruleViewToDraft(rule: AlertRuleView): RuleDraft {
  return {
    ...rule,
    uid: `rule-uid-${nextSeq()}`,
    node_ids: [...rule.node_ids],
    tags: [...rule.tags],
    delivery: [...rule.delivery],
  };
}

/** Server View → editable draft. Copies arrays (no shared refs) and seeds the
 * transient secret fields blank; stored secrets are never echoed. */
export function viewToDraft(view: AlertSettingsView): AlertsDraft {
  return {
    enabled: view.enabled,
    smtp: {
      enabled: view.smtp.enabled,
      host: view.smtp.host,
      port: view.smtp.port,
      username: view.smtp.username,
      sender: view.smtp.sender,
      recipients: [...view.smtp.recipients],
      transport: view.smtp.transport,
      send_resolved: view.smtp.send_resolved,
      password_configured: view.smtp.password_configured,
      password: '',
      clear_password: false,
    },
    webhook: {
      enabled: view.webhook.enabled,
      url: view.webhook.url,
      send_resolved: view.webhook.send_resolved,
      secret_configured: view.webhook.secret_configured,
      secret: '',
      clear_secret: false,
    },
    rules: view.rules.map(ruleViewToDraft),
    inspection: { ...view.inspection, delivery: [...view.inspection.delivery] },
  };
}

function ruleToPayload(rule: RuleDraft): UpdateAlertRuleRequest {
  return {
    id: rule.id,
    name: rule.name,
    enabled: rule.enabled,
    metric: rule.metric,
    comparator: rule.comparator,
    threshold: rule.threshold,
    window_minutes: rule.window_minutes,
    severity: rule.severity,
    scope_mode: rule.scope_mode,
    node_ids: [...rule.node_ids],
    tags: [...rule.tags],
    delivery: [...rule.delivery],
    cooldown_minutes: rule.cooldown_minutes,
    send_resolved: rule.send_resolved,
  };
}

/**
 * Draft + reauth → POST payload. The `password`/`secret` keys are included
 * ONLY when a non-blank value was typed and the field isn't being cleared —
 * omitting the key (rather than sending null/undefined) lets the server keep the
 * stored secret. `current_password`/`code` are likewise omitted when blank.
 */
export function draftToPayload(draft: AlertsDraft, reauth: ReauthInput): UpdateAlertSettingsRequest {
  const sendPassword = !draft.smtp.clear_password && draft.smtp.password.trim() !== '';
  const smtp: UpdateAlertSmtpSettingsRequest = {
    enabled: draft.smtp.enabled,
    host: draft.smtp.host.trim(),
    port: draft.smtp.port,
    username: draft.smtp.username.trim(),
    sender: draft.smtp.sender.trim(),
    recipients: [...draft.smtp.recipients],
    transport: draft.smtp.transport,
    send_resolved: draft.smtp.send_resolved,
    clear_password: draft.smtp.clear_password,
    ...(sendPassword ? { password: draft.smtp.password } : {}),
  };

  const sendSecret = !draft.webhook.clear_secret && draft.webhook.secret.trim() !== '';
  const webhook: UpdateAlertWebhookSettingsRequest = {
    enabled: draft.webhook.enabled,
    url: draft.webhook.url.trim(),
    send_resolved: draft.webhook.send_resolved,
    clear_secret: draft.webhook.clear_secret,
    ...(sendSecret ? { secret: draft.webhook.secret } : {}),
  };

  const currentPassword = reauth.current_password;
  const code = reauth.code.trim();
  return {
    ...(currentPassword.trim() !== '' ? { current_password: currentPassword } : {}),
    ...(code !== '' ? { code } : {}),
    enabled: draft.enabled,
    smtp,
    webhook,
    rules: draft.rules.map(ruleToPayload),
    inspection: { ...draft.inspection, delivery: [...draft.inspection.delivery] },
  };
}
