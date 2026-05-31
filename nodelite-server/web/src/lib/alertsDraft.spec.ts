import { describe, expect, it } from 'vitest';
import type { AlertSettingsView } from '@/api';
import {
  blankRule,
  draftToPayload,
  emptyAlertsConfig,
  viewToDraft,
  type ReauthInput,
} from './alertsDraft';

const NO_REAUTH: ReauthInput = { current_password: '', code: '' };

function sampleView(): AlertSettingsView {
  return {
    enabled: true,
    smtp: {
      enabled: true,
      host: 'smtp.example.com',
      port: 465,
      username: 'mailer',
      sender: 'alerts@example.com',
      recipients: ['ops@example.com', 'sre@example.com'],
      transport: 'tls',
      send_resolved: false,
      password_configured: true,
    },
    webhook: {
      enabled: true,
      url: 'https://hooks.example.com/x',
      send_resolved: true,
      secret_configured: true,
    },
    rules: [
      {
        id: 'cpu-hot',
        name: 'CPU hot',
        enabled: true,
        metric: 'cpu_usage_percent',
        comparator: 'gt',
        threshold: 90,
        window_minutes: 10,
        severity: 'critical',
        scope_mode: 'tags',
        node_ids: [],
        tags: ['prod'],
        delivery: ['smtp', 'webhook'],
        cooldown_minutes: 15,
        send_resolved: true,
      },
    ],
    inspection: {
      enabled: true,
      local_time: '08:30',
      lookback_hours: 12,
      delivery: ['webhook'],
      offline_grace_minutes: 5,
      latency_warn_ms: 300,
      cpu_warn_percent: 80,
      memory_warn_percent: 85,
    },
  };
}

describe('emptyAlertsConfig / blankRule', () => {
  it('emptyAlertsConfig has legacy defaults and blank transient secrets', () => {
    const cfg = emptyAlertsConfig();
    expect(cfg.enabled).toBe(false);
    expect(cfg.smtp.port).toBe(587);
    expect(cfg.smtp.transport).toBe('start_tls');
    expect(cfg.smtp.password).toBe('');
    expect(cfg.smtp.clear_password).toBe(false);
    expect(cfg.webhook.secret).toBe('');
    expect(cfg.inspection.delivery).toEqual(['smtp']);
    expect(cfg.rules).toEqual([]);
  });

  it('blankRule gives unique uid + id each call', () => {
    const a = blankRule();
    const b = blankRule();
    expect(a.uid).not.toBe(b.uid);
    expect(a.id).not.toBe(b.id);
    expect(a.metric).toBe('cpu_usage_percent');
    expect(a.delivery).toEqual(['smtp']);
  });
});

describe('viewToDraft', () => {
  it('copies fields, seeds blank secrets, and assigns rule uids', () => {
    const view = sampleView();
    const draft = viewToDraft(view);
    expect(draft.enabled).toBe(true);
    expect(draft.smtp.host).toBe('smtp.example.com');
    expect(draft.smtp.password_configured).toBe(true);
    // stored secret never echoed
    expect(draft.smtp.password).toBe('');
    expect(draft.smtp.clear_password).toBe(false);
    expect(draft.webhook.secret_configured).toBe(true);
    expect(draft.webhook.secret).toBe('');
    expect(draft.rules).toHaveLength(1);
    expect(draft.rules[0]?.uid).toMatch(/^rule-uid-/);
    expect(draft.rules[0]?.id).toBe('cpu-hot');
  });

  it('clones arrays so editing the draft never mutates the source view', () => {
    const view = sampleView();
    const draft = viewToDraft(view);
    draft.smtp.recipients.push('extra@example.com');
    draft.rules[0]?.tags.push('staging');
    draft.inspection.delivery.push('smtp');
    expect(view.smtp.recipients).toEqual(['ops@example.com', 'sre@example.com']);
    expect(view.rules[0]?.tags).toEqual(['prod']);
    expect(view.inspection.delivery).toEqual(['webhook']);
  });
});

describe('draftToPayload — secret keep/clear', () => {
  it('keep: blank password (configured) omits the key, clear flag false', () => {
    const draft = viewToDraft(sampleView()); // password '', clear false
    const payload = draftToPayload(draft, NO_REAUTH);
    expect('password' in payload.smtp).toBe(false);
    expect(payload.smtp.clear_password).toBe(false);
    expect('secret' in payload.webhook).toBe(false);
    expect(payload.webhook.clear_secret).toBe(false);
  });

  it('set: a typed password is sent verbatim with clear flag false', () => {
    const draft = viewToDraft(sampleView());
    draft.smtp.password = 's3cr3t ';
    draft.webhook.secret = 'hook-key';
    const payload = draftToPayload(draft, NO_REAUTH);
    // sent untrimmed so passwords with spaces survive
    expect(payload.smtp.password).toBe('s3cr3t ');
    expect(payload.smtp.clear_password).toBe(false);
    expect(payload.webhook.secret).toBe('hook-key');
  });

  it('clear: clear flag wins even when a value was typed (key omitted)', () => {
    const draft = viewToDraft(sampleView());
    draft.smtp.password = 'ignored';
    draft.smtp.clear_password = true;
    draft.webhook.secret = 'ignored';
    draft.webhook.clear_secret = true;
    const payload = draftToPayload(draft, NO_REAUTH);
    expect('password' in payload.smtp).toBe(false);
    expect(payload.smtp.clear_password).toBe(true);
    expect('secret' in payload.webhook).toBe(false);
    expect(payload.webhook.clear_secret).toBe(true);
  });

  it('whitespace-only secret counts as keep (omitted), matching the server filter', () => {
    const draft = viewToDraft(sampleView());
    draft.smtp.password = '   ';
    const payload = draftToPayload(draft, NO_REAUTH);
    expect('password' in payload.smtp).toBe(false);
  });
});

describe('draftToPayload — reauth + rules + scalars', () => {
  it('includes current_password / code only when non-blank', () => {
    const draft = emptyAlertsConfig();
    expect('current_password' in draftToPayload(draft, NO_REAUTH)).toBe(false);
    expect('code' in draftToPayload(draft, NO_REAUTH)).toBe(false);
    const withPw = draftToPayload(draft, { current_password: 'pw', code: '' });
    expect(withPw.current_password).toBe('pw');
    expect('code' in withPw).toBe(false);
    const withCode = draftToPayload(draft, { current_password: '', code: '123456' });
    expect(withCode.code).toBe('123456');
    expect('current_password' in withCode).toBe(false);
  });

  it('strips uid from rules and preserves every rule field', () => {
    const draft = viewToDraft(sampleView());
    const payload = draftToPayload(draft, NO_REAUTH);
    expect(payload.rules).toHaveLength(1);
    const rule = payload.rules[0];
    expect(rule).not.toHaveProperty('uid');
    expect(rule?.id).toBe('cpu-hot');
    expect(rule?.scope_mode).toBe('tags');
    expect(rule?.tags).toEqual(['prod']);
    expect(rule?.delivery).toEqual(['smtp', 'webhook']);
    expect(rule?.threshold).toBe(90);
  });

  it('passes through enabled + numeric scalars and trims text fields', () => {
    const draft = viewToDraft(sampleView());
    draft.smtp.host = '  smtp.trim.me  ';
    const payload = draftToPayload(draft, NO_REAUTH);
    expect(payload.enabled).toBe(true);
    expect(payload.smtp.host).toBe('smtp.trim.me');
    expect(payload.smtp.port).toBe(465);
    expect(payload.inspection.local_time).toBe('08:30');
    expect(payload.inspection.lookback_hours).toBe(12);
  });
});
