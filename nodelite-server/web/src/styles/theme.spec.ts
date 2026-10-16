import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const theme = readFileSync(resolve('src/styles/theme.css'), 'utf8');

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
