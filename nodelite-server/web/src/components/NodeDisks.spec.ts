import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { mount } from '@vue/test-utils';
import { createApp, defineComponent, h } from 'vue';
import { setupI18n, getI18n, __resetI18nForTest } from '@/i18n';
import { makeNodeStatus } from '@/api/__fixtures__/nodes';
import type { DiskUsage } from '@/api';
import NodeDisks from './NodeDisks.vue';

const FAKE_DICT = {
  en: {
    'node.no_disks': 'No disk metrics reported yet.',
    'node.disk.device': 'Device',
    'node.disk.mount': 'Mount',
    'node.disk.filesystem': 'Filesystem',
    'node.disk.usage': 'Usage',
    'node.disk.capacity': 'Capacity',
  },
  'zh-CN': { 'node.no_disks': '暂无磁盘指标。' },
};

const Stub = defineComponent({ render: () => h('div') });

function disk(over: Partial<DiskUsage>): DiskUsage {
  return {
    device: '/dev/sda1',
    mount_point: '/',
    fs_type: 'ext4',
    total_bytes: 100_000_000_000,
    available_bytes: 60_000_000_000,
    used_bytes: 40_000_000_000,
    used_percent: 40,
    ...over,
  };
}

function mountDisks(node: ReturnType<typeof makeNodeStatus> | null) {
  return mount(NodeDisks, { props: { node }, global: { plugins: [getI18n()] } });
}

describe('NodeDisks', () => {
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

  it('shows the empty placeholder when there are no disks', () => {
    const node = makeNodeStatus({ snapshot: null });
    const wrapper = mountDisks(node);
    expect(wrapper.find('[data-test="node-disks-empty"]').exists()).toBe(true);
  });

  it('renders one row per unique disk with usage + capacity', () => {
    const node = makeNodeStatus();
    node.snapshot!.disks = [
      disk({ device: '/dev/sda1', total_bytes: 100, used_percent: 40 }),
      disk({ device: '/dev/sda1', total_bytes: 100 }), // dup → filtered
      disk({ device: '/dev/sdb1', total_bytes: 200, used_percent: 95 }),
    ];
    const wrapper = mountDisks(node);
    const rows = wrapper.findAll('[data-test="disk-row"]');
    expect(rows).toHaveLength(2);
    expect(wrapper.text()).toContain('40%');
    expect(wrapper.text()).toContain('95%');
    // high usage gets the bad severity class
    expect(wrapper.find('.disks-bar > span.bad').exists()).toBe(true);
  });
});
