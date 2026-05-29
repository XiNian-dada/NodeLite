/**
 * vue-i18n setup. Loads the legacy ui-i18n.json at runtime (Vite proxies
 * /assets/ui-i18n.json to the Rust backend; in production the Rust route
 * serves the same file). We keep dotted keys (e.g. "common.theme_toggle")
 * via flatJson: true.
 */

import { createI18n } from 'vue-i18n';
import type { App } from 'vue';

export const SUPPORTED_LOCALES = ['en', 'zh-CN'] as const;
export type SupportedLocale = (typeof SUPPORTED_LOCALES)[number];

export const FALLBACK_LOCALE: SupportedLocale = 'en';
export const LANGUAGE_STORAGE_KEY = 'nodelite.ui.language';
export const I18N_ASSET_PATH = '/assets/ui-i18n.json';

export type Messages = Record<string, string>;
export type Dictionary = Record<SupportedLocale, Messages>;

export type AppI18n = ReturnType<typeof createI18n>;

let i18nInstance: AppI18n | null = null;

export function isSupportedLocale(value: string): value is SupportedLocale {
  return (SUPPORTED_LOCALES as readonly string[]).includes(value);
}

/**
 * Resolve the initial locale: localStorage > navigator.language prefix > fallback.
 * Matches the legacy resolveLanguage() behavior in assets/index.html.
 */
export function resolveInitialLocale(): SupportedLocale {
  try {
    const stored = window.localStorage.getItem(LANGUAGE_STORAGE_KEY);
    if (stored !== null && isSupportedLocale(stored)) {
      return stored;
    }
  } catch {
    /* localStorage unavailable */
  }
  const nav = (typeof navigator !== 'undefined' ? navigator.language : '') || '';
  if (isSupportedLocale(nav)) return nav;
  const prefix = nav.split('-')[0];
  for (const locale of SUPPORTED_LOCALES) {
    if (locale === prefix || locale.startsWith(`${prefix}-`)) {
      return locale;
    }
  }
  return FALLBACK_LOCALE;
}

async function fetchDictionary(): Promise<Dictionary> {
  const res = await fetch(I18N_ASSET_PATH, {
    credentials: 'same-origin',
    headers: { Accept: 'application/json' },
  });
  if (!res.ok) {
    throw new Error(`failed to load ui-i18n.json: ${res.status}`);
  }
  const raw = (await res.json()) as Record<string, Messages>;
  const out: Partial<Dictionary> = {};
  for (const locale of SUPPORTED_LOCALES) {
    out[locale] = raw[locale] ?? {};
  }
  return out as Dictionary;
}

/**
 * Build the vue-i18n instance, fetch the dictionary, register it on the app.
 * Errors propagate — main.ts wraps this in try/catch so the SPA still mounts
 * with an empty dictionary if the network fetch fails.
 */
export async function setupI18n(app: App): Promise<AppI18n> {
  const messages = await fetchDictionary();
  const instance = createI18n({
    legacy: false,
    globalInjection: true,
    locale: resolveInitialLocale(),
    fallbackLocale: FALLBACK_LOCALE,
    flatJson: true,
    messages,
  }) as unknown as AppI18n;
  i18nInstance = instance;
  app.use(instance);
  return instance;
}

export function getI18n(): AppI18n {
  if (i18nInstance === null) {
    throw new Error('setupI18n must be called before getI18n');
  }
  return i18nInstance;
}

/** Test-only: reset module-level state between specs. */
export function __resetI18nForTest(): void {
  i18nInstance = null;
}
