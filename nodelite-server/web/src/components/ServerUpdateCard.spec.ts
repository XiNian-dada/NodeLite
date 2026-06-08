import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';
import type { VueWrapper } from '@vue/test-utils';
import { createApp, defineComponent, h } from 'vue';
import { setupI18n, getI18n, __resetI18nForTest } from '@/i18n';
import { apiClient } from '@/api';
import { ApiError } from '@/api/client';
import { makeSettings } from '@/api/__fixtures__/nodes';
import ServerUpdateCard from './ServerUpdateCard.vue';

vi.mock('@/api', async () => {
  const actual = await vi.importActual<typeof import('@/api')>('@/api');
  return {
    ...actual,
    apiClient: { ...actual.apiClient, updateServer: vi.fn(), serverUpdateLog: vi.fn() },
  };
});

const mockUpdate = vi.mocked(apiClient.updateServer);
const mockUpdateLog = vi.mocked(apiClient.serverUpdateLog);
const mountedWrappers: VueWrapper[] = [];

const FAKE_DICT = {
  en: {
    'settings.version.title': 'Version & Updates',
    'settings.version.current': 'Current version',
    'settings.version.repository': 'Repository',
    'settings.version.public_url': 'Public URL',
    'settings.version.listen': 'Listen',
    'settings.version.check_updates': 'Check updates',
    'settings.version.view_update_log': 'View update log',
    'settings.version.open_release': 'Releases',
    'settings.version.checking': 'Checking…',
    'settings.version.update_available': 'New: {version}',
    'settings.version.up_to_date': 'Up to date: {version}',
    'settings.version.check_failed': 'Check failed: {error}',
    'settings.version.manual_update_note_2fa': 'Enter code',
    'settings.version.manual_update_note_password': 'Enter password',
    'settings.version.update_now': 'Update now',
    'settings.version.update_starting': 'Starting…',
    'settings.version.update_started': 'Started',
    'settings.version.update_failed': 'Failed: {error}',
    'settings.version.console_title': 'Server Update Console',
    'settings.version.console_subtitle': 'Live installer output',
    'settings.version.console_refresh': 'Refresh',
    'settings.version.console_close': 'Close',
    'settings.version.console_status_idle': 'Idle',
    'settings.version.console_status_waiting': 'Launching',
    'settings.version.console_status_running': 'Streaming',
    'settings.version.console_status_retrying': 'Reconnecting',
    'settings.version.console_status_error': 'Failed',
    'settings.version.console_empty': 'No update log loaded yet.',
    'settings.version.console_preparing': 'Preparing update request…',
    'settings.version.console_connecting': 'Connecting to live update log…',
    'settings.version.console_failed_to_start': 'The update process did not start successfully.',
    'settings.version.console_loaded': 'Loaded {size} of update output',
    'settings.version.console_fetch_failed': 'Log stream unavailable: {error}',
    'settings.version.console_following': 'Following latest output',
    'settings.version.console_paused': 'Scroll to the bottom to resume following',
    'settings.summary.version': 'Current Version',
    'settings.password.current': 'Current password',
    'settings.security.verification_code': '6-digit code',
  },
  'zh-CN': {},
};

const Stub = defineComponent({ render: () => h('div') });

// fetch routes: ui-i18n.json → dict; api.github.com → release.
function routedFetch(release: { tag_name: string } | { fail: true }) {
  return vi.fn().mockImplementation((url: string) => {
    if (String(url).includes('ui-i18n.json')) {
      return Promise.resolve({
        ok: true,
        status: 200,
        json: () => Promise.resolve(FAKE_DICT),
      } as unknown as Response);
    }
    if ('fail' in release)
      return Promise.resolve({ ok: false, status: 503 } as unknown as Response);
    return Promise.resolve({
      ok: true,
      status: 200,
      json: () => Promise.resolve(release),
    } as unknown as Response);
  });
}

async function mountCard(
  over = {},
  release: { tag_name: string } | { fail: true } = { tag_name: 'v2.3.0' },
) {
  vi.stubGlobal('fetch', routedFetch(release));
  const dummy = createApp(Stub);
  await setupI18n(dummy);
  const settings = makeSettings({
    server_version: '2.3.0',
    repository: 'https://github.com/o/r',
    ...over,
  });
  const wrapper = mount(ServerUpdateCard, {
    props: { settings },
    global: { plugins: [getI18n()] },
  });
  mountedWrappers.push(wrapper);
  return wrapper;
}

describe('ServerUpdateCard', () => {
  beforeEach(() => {
    __resetI18nForTest();
    mockUpdate.mockReset();
    mockUpdateLog.mockReset();
  });
  afterEach(() => {
    mountedWrappers.splice(0).forEach((wrapper) => wrapper.unmount());
    __resetI18nForTest();
    vi.unstubAllGlobals();
    vi.clearAllMocks();
  });

  it('shows the current version', async () => {
    const wrapper = await mountCard();
    expect(wrapper.find('[data-test="server-version"]').text()).toBe('2.3.0');
  });

  it('check-update reports up-to-date for an equal latest tag', async () => {
    const wrapper = await mountCard({}, { tag_name: 'v2.3.0' });
    await wrapper.find('[data-test="check-update"]').trigger('click');
    await flushPromises();
    expect(wrapper.find('[data-test="settings-message"]').text()).toContain('Up to date: 2.3.0');
  });

  it('check-update reports a newer release', async () => {
    const wrapper = await mountCard({}, { tag_name: 'v2.4.0' });
    await wrapper.find('[data-test="check-update"]').trigger('click');
    await flushPromises();
    expect(wrapper.find('[data-test="settings-message"]').text()).toContain('New: 2.4.0');
  });

  it('posts a server update with reauth and shows the started message', async () => {
    mockUpdate.mockResolvedValueOnce({ ok: true, message: '' });
    mockUpdateLog.mockResolvedValueOnce({
      exists: true,
      offset: 0,
      next_offset: 18,
      text: 'installer ready',
    });
    const wrapper = await mountCard(); // 2FA off → password field
    await wrapper.find('[data-test="reauth-password"]').setValue('pw');
    await wrapper.find('[data-test="server-update-form"]').trigger('submit');
    await flushPromises();
    expect(mockUpdate).toHaveBeenCalledWith({ current_password: 'pw' });
    expect(
      wrapper.find('[data-test="server-update-form"] [data-test="settings-message"]').text(),
    ).toContain('Started');
    expect(wrapper.find('[data-test="update-console-modal"]').exists()).toBe(true);
    expect(wrapper.find('[data-test="update-console-log"]').text()).toContain('installer ready');
  });

  it('surfaces the server error message on failure', async () => {
    mockUpdate.mockRejectedValueOnce(
      new ApiError(400, JSON.stringify({ ok: false, message: 'bad password' })),
    );
    const wrapper = await mountCard();
    await wrapper.find('[data-test="reauth-password"]').setValue('nope');
    await wrapper.find('[data-test="server-update-form"]').trigger('submit');
    await flushPromises();
    expect(
      wrapper.find('[data-test="server-update-form"] [data-test="settings-message"]').text(),
    ).toContain('bad password');
    expect(wrapper.find('[data-test="update-console-modal"]').exists()).toBe(true);
    expect(wrapper.find('[data-test="update-console-log"]').text()).toContain('bad password');
  });

  it('opens the update console and fetches existing log output', async () => {
    mockUpdateLog.mockResolvedValueOnce({
      exists: true,
      offset: 0,
      next_offset: 12,
      text: 'line one',
    });
    const wrapper = await mountCard();
    await wrapper.find('[data-test="view-update-log"]').trigger('click');
    await flushPromises();
    expect(mockUpdateLog).toHaveBeenCalledWith(0);
    expect(wrapper.find('[data-test="update-console-modal"]').exists()).toBe(true);
    expect(wrapper.find('[data-test="update-console-log"]').text()).toContain('line one');
  });
});
