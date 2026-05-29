import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createApp } from 'vue';
import {
  FALLBACK_LOCALE,
  LANGUAGE_STORAGE_KEY,
  __resetI18nForTest,
  resolveInitialLocale,
  setupI18n,
} from './index';
import { useLanguage } from './language';

const FAKE_DICT = {
  en: { 'common.theme_toggle': 'Toggle theme' },
  'zh-CN': { 'common.theme_toggle': '切换主题' },
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
});
