import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const theme = readFileSync(resolve('src/styles/theme.css'), 'utf8');
const alerts = readFileSync(resolve('src/views/AlertsView.vue'), 'utf8');

describe('shared select styling', () => {
  it('reserves an inset arrow gutter independently of component field padding', () => {
    const rule = theme.match(/:root select:not\(\[multiple\]\):not\(\[size\]\)\s*\{([^}]+)\}/)?.[1];
    expect(rule).toBeDefined();
    expect(rule).toContain('appearance: none');
    expect(rule).toContain('padding-inline-end: 36px');
    expect(rule).toContain('right 14px center');
    expect(rule).toContain('background-repeat: no-repeat');
  });

  it('keeps a native arrow available in forced-color mode', () => {
    expect(theme).toMatch(/@media \(forced-colors: active\)[\s\S]*appearance: auto/);
  });
});

describe('delivery channel layout', () => {
  it('stretches sibling cards instead of preserving mismatched content heights', () => {
    expect(alerts).toMatch(/\.alerts__grid--channels\s*\{[^}]*align-items: stretch/);
    for (const component of ['SmtpChannelCard', 'WebhookChannelCard']) {
      const source = readFileSync(resolve(`src/components/${component}.vue`), 'utf8');
      expect(source).toMatch(/\.card-body\s*\{[^}]*flex: 1/);
      expect(source).toMatch(/\.card-actions\s*\{[^}]*margin-top: auto/);
    }
  });
});
