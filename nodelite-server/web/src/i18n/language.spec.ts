import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createApp, defineComponent, h } from 'vue';
import { createI18n, useI18n, type Composer } from 'vue-i18n';
import {
  FALLBACK_LOCALE,
  LANGUAGE_STORAGE_KEY,
  __resetI18nForTest,
  resolveInitialLocale,
  setupI18n,
} from './index';
import { useLanguage } from './language';

const FAKE_DICT = {
  en: {
    'common.theme_toggle': 'Toggle theme',
    'common.greeting': 'Hello, {name}!',
    'common.fallback_only': 'English fallback',
  },
  'zh-CN': {
    'common.theme_toggle': '切换主题',
    'common.greeting': '你好，{name}！',
  },
};

const TestRoot = { render: () => null };

function stubDictionaryFetch(body: unknown = FAKE_DICT, ok = true): void {
  vi.stubGlobal(
    'fetch',
    vi.fn().mockResolvedValue({
      ok,
      status: ok ? 200 : 500,
      json: () => Promise.resolve(body),
    } as unknown as Response),
  );
}

function mockNavigatorLanguage(value: string): void {
  Object.defineProperty(navigator, 'language', {
    configurable: true,
    value,
  });
}

describe('resolveInitialLocale', () => {
  beforeEach(() => {
    window.localStorage.clear();
    mockNavigatorLanguage('en-US');
  });

  afterEach(() => {
    window.localStorage.clear();
  });

  it('honors a supported localStorage value', () => {
    window.localStorage.setItem(LANGUAGE_STORAGE_KEY, 'zh-CN');
    expect(resolveInitialLocale()).toBe('zh-CN');
  });

  it('matches navigator.language exactly', () => {
    mockNavigatorLanguage('zh-CN');
    expect(resolveInitialLocale()).toBe('zh-CN');
  });

  it('matches by prefix when navigator.language is a generic tag', () => {
    mockNavigatorLanguage('zh');
    expect(resolveInitialLocale()).toBe('zh-CN');
  });

  it('falls back to en for unknown navigator.language', () => {
    mockNavigatorLanguage('fr-FR');
    expect(resolveInitialLocale()).toBe(FALLBACK_LOCALE);
  });

  it('ignores unsupported stored value and falls back to navigator chain', () => {
    window.localStorage.setItem(LANGUAGE_STORAGE_KEY, 'klingon');
    mockNavigatorLanguage('en-GB');
    expect(resolveInitialLocale()).toBe('en');
  });
});

describe('useLanguage', () => {
  beforeEach(() => {
    window.localStorage.clear();
    mockNavigatorLanguage('en-US');
    __resetI18nForTest();
    stubDictionaryFetch();
  });

  afterEach(() => {
    window.localStorage.clear();
    __resetI18nForTest();
    vi.unstubAllGlobals();
  });

  it('reflects the current locale and flips it on setLocale', async () => {
    const app = createApp(TestRoot);
    await setupI18n(app);
    const { currentLocale, setLocale } = useLanguage();

    expect(currentLocale.value).toBe('en');

    setLocale('zh-CN');
    expect(currentLocale.value).toBe('zh-CN');
    expect(window.localStorage.getItem(LANGUAGE_STORAGE_KEY)).toBe('zh-CN');
  });

  it('falls back to en when an unsupported locale is passed', async () => {
    const app = createApp(TestRoot);
    await setupI18n(app);
    const { currentLocale, setLocale } = useLanguage();

    setLocale('klingon');
    expect(currentLocale.value).toBe('en');
    expect(window.localStorage.getItem(LANGUAGE_STORAGE_KEY)).toBe('en');
  });

  it('preserves dotted keys, interpolation and fallback when switching languages', async () => {
    const instance = await setupI18n(createApp(TestRoot));
    expect(instance.mode).toBe('composition');
    const composer = instance.global as Composer;
    const { setLocale } = useLanguage();

    expect(composer.t('common.theme_toggle')).toBe('Toggle theme');
    expect(composer.t('common.greeting', { name: 'NodeLite' })).toBe('Hello, NodeLite!');

    setLocale('zh-CN');
    expect(composer.t('common.theme_toggle')).toBe('切换主题');
    expect(composer.t('common.greeting', { name: 'NodeLite' })).toBe('你好，NodeLite！');
    expect(composer.t('common.fallback_only')).toBe('English fallback');

    setLocale('en');
    expect(composer.t('common.theme_toggle')).toBe('Toggle theme');
  });
});

describe('document language', () => {
  beforeEach(() => {
    window.localStorage.clear();
    mockNavigatorLanguage('en-US');
    document.documentElement.lang = 'en';
    __resetI18nForTest();
    stubDictionaryFetch();
  });

  afterEach(() => {
    window.localStorage.clear();
    document.documentElement.lang = 'en';
    __resetI18nForTest();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it('applies the resolved initial locale', async () => {
    window.localStorage.setItem(LANGUAGE_STORAGE_KEY, 'zh-CN');
    await setupI18n(createApp(TestRoot));
    expect(document.documentElement.lang).toBe('zh-CN');
  });

  it('applies the initial locale when the dictionary fails to load', async () => {
    window.localStorage.setItem(LANGUAGE_STORAGE_KEY, 'zh-CN');
    vi.spyOn(console, 'error').mockImplementation(() => {});
    vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new Error('offline')));
    await setupI18n(createApp(TestRoot));
    expect(document.documentElement.lang).toBe('zh-CN');
  });

  it('updates immediately on setLocale, including the fallback for unknown locales', async () => {
    await setupI18n(createApp(TestRoot));
    const { setLocale } = useLanguage();

    setLocale('zh-CN');
    expect(document.documentElement.lang).toBe('zh-CN');

    setLocale('klingon');
    expect(document.documentElement.lang).toBe('en');
  });
});

describe('setupI18n dictionary failure', () => {
  const UsesI18n = defineComponent({
    setup() {
      const { t } = useI18n();
      const { currentLocale } = useLanguage();
      return () => h('p', `${currentLocale.value}:${t('common.theme_toggle')}`);
    },
  });

  beforeEach(() => {
    window.localStorage.clear();
    mockNavigatorLanguage('en-US');
    __resetI18nForTest();
    vi.spyOn(console, 'error').mockImplementation(() => {});
  });

  afterEach(() => {
    __resetI18nForTest();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it.each([
    ['network failure', () => vi.fn().mockRejectedValue(new Error('offline'))],
    ['HTTP 500', () => vi.fn().mockResolvedValue({ ok: false, status: 500 })],
    [
      'corrupt JSON',
      () =>
        vi.fn().mockResolvedValue({
          ok: true,
          status: 200,
          json: () => Promise.reject(new SyntaxError('bad json')),
        }),
    ],
    [
      'JSON null',
      () => vi.fn().mockResolvedValue({ ok: true, status: 200, json: () => Promise.resolve(null) }),
    ],
  ])('mounts a useI18n component after %s', async (_name, makeFetch) => {
    vi.stubGlobal('fetch', makeFetch());
    const app = createApp(UsesI18n);
    await setupI18n(app);

    const host = document.createElement('div');
    app.mount(host);

    expect(host.textContent).toBe('en:common.theme_toggle');
    expect(console.error).toHaveBeenCalled();
    app.unmount();
  });
});

describe('translation HTML sanitization', () => {
  it.each([
    ['<iframe srcdoc="{content}"></iframe>', { content: '<b>untrusted</b>' }],
    ['<iframe srcdoc="<b>untrusted</b>"></iframe>', {}],
  ])('neutralizes srcdoc in %s (GHSA-83rm-6c8r-g728)', (message, parameters) => {
    const instance = createI18n({
      legacy: false,
      locale: 'en',
      escapeParameter: true,
      warnHtmlMessage: false,
      messages: { en: { message } },
    });
    const template = document.createElement('template');
    template.innerHTML = instance.global.t('message', parameters);

    const iframe = template.content.querySelector('iframe');
    expect(iframe).not.toBeNull();
    expect(iframe?.hasAttribute('srcdoc')).toBe(false);
  });
});
