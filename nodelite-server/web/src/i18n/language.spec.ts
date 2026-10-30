import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createApp } from 'vue';
import { createI18n, type Composer } from 'vue-i18n';
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
