import { describe, expect, it } from 'vitest';
import { effectScope } from 'vue';
import { useChartSelection, PRESET_WINDOWS } from './useChartSelection';

function run() {
  const scope = effectScope();
  const sel = scope.run(() => useChartSelection())!;
  return { sel, scope };
}

describe('useChartSelection', () => {
  it('defaults to the 24h preset', () => {
    const { sel, scope } = run();
    expect(sel.activeKey.value).toBe('last_24h');
    expect(sel.windowHours.value).toBe(24);
    scope.stop();
  });

  it('maps each preset to its window hours', () => {
    const { sel, scope } = run();
    for (const p of PRESET_WINDOWS) {
      sel.selectPreset(p.key);
      expect(sel.windowHours.value).toBe(p.hours);
    }
    scope.stop();
  });
});
