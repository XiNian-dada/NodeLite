import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { mount } from '@vue/test-utils';
import { createApp, defineComponent, h } from 'vue';
import { setupI18n, getI18n, __resetI18nForTest } from '@/i18n';
import { makeLogEntry } from '@/api/__fixtures__/nodes';
import LogPanel from './LogPanel.vue';

const FAKE_DICT = {
  en: {
    'node.logs.empty': 'No logs yet.',
    'node.logs.load_failed': 'Failed: {error}',
    'node.logs.level_info': 'Info',
    'node.logs.level_warn': 'Warn',
    'node.logs.level_error': 'Error',
  },
  'zh-CN': { 'node.logs.empty': '暂无日志。' },
};

const Stub = defineComponent({ render: () => h('div') });

function mountPanel(props: { entries: ReturnType<typeof makeLogEntry>[]; error: Error | null }) {
  return mount(LogPanel, { props, global: { plugins: [getI18n()] } });
}

describe('LogPanel', () => {
  beforeEach(async () => {
    __resetI18nForTest();
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({
        ok: true,
        status: 200,
        json: () => Promise.resolve(FAKE_DICT),
      } as unknown as Response),
    );
    const dummy = createApp(Stub);
    await setupI18n(dummy);
  });

  afterEach(() => {
    __resetI18nForTest();
    vi.unstubAllGlobals();
  });

  it('shows the empty state with no entries', () => {
    const wrapper = mountPanel({ entries: [], error: null });
    expect(wrapper.find('[data-test="log-empty"]').exists()).toBe(true);
  });

  it('shows the error state', () => {
    const wrapper = mountPanel({ entries: [], error: new Error('boom') });
    expect(wrapper.find('[data-test="log-error"]').text()).toContain('boom');
  });

  it('renders entries newest-first with level classes', () => {
    const wrapper = mountPanel({
      entries: [
        makeLogEntry({ level: 'info', message: 'first' }),
        makeLogEntry({ level: 'error', message: 'second' }),
      ],
      error: null,
    });
    const entries = wrapper.findAll('[data-test="log-entry"]');
    expect(entries).toHaveLength(2);
    // reversed → 'second' (error) first
    expect(entries[0]!.text()).toContain('second');
    expect(entries[0]!.find('.log-level').classes()).toContain('error');
    expect(entries[1]!.text()).toContain('first');
  });
});
