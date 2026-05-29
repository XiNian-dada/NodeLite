import { computed, type ComputedRef } from 'vue';
import {
  FALLBACK_LOCALE,
  LANGUAGE_STORAGE_KEY,
  SUPPORTED_LOCALES,
  getI18n,
  isSupportedLocale,
  type SupportedLocale,
} from './index';

interface LocaleRef {
  value: string;
}

export function useLanguage(): {
  currentLocale: ComputedRef<SupportedLocale>;
  setLocale: (locale: string) => void;
  supportedLocales: readonly SupportedLocale[];
} {
  const i18n = getI18n();
  const localeRef = i18n.global.locale as unknown as LocaleRef;
  const currentLocale = computed(() => {
    const raw = localeRef.value;
    return isSupportedLocale(raw) ? raw : FALLBACK_LOCALE;
  });

  function setLocale(locale: string): void {
    const next = isSupportedLocale(locale) ? locale : FALLBACK_LOCALE;
    localeRef.value = next;
    try {
      window.localStorage.setItem(LANGUAGE_STORAGE_KEY, next);
    } catch {
      /* localStorage unavailable */
    }
  }

  return { currentLocale, setLocale, supportedLocales: SUPPORTED_LOCALES };
}
