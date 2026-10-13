import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { APPEARANCE_STORAGE_KEY, THEME_STORAGE_KEY, setupTheme, useTheme } from './useTheme';

describe('useTheme', () => {
  beforeEach(() => {
    window.localStorage.clear();
    delete document.documentElement.dataset.theme;
    delete document.documentElement.dataset.appearance;
  });

  afterEach(() => {
    vi.restoreAllMocks();
    window.localStorage.clear();
    delete document.documentElement.dataset.theme;
    delete document.documentElement.dataset.appearance;
  });

  describe('setupTheme', () => {
    it('honors stored "light"', () => {
      window.localStorage.setItem(THEME_STORAGE_KEY, 'light');
      expect(setupTheme()).toBe('light');
      expect(document.documentElement.dataset.theme).toBe('light');
    });

    it('honors stored "dark"', () => {
      window.localStorage.setItem(THEME_STORAGE_KEY, 'dark');
      expect(setupTheme()).toBe('dark');
      expect(document.documentElement.dataset.theme).toBe('dark');
    });

    it('falls back to dark when no value stored', () => {
      expect(setupTheme()).toBe('dark');
      expect(document.documentElement.dataset.theme).toBe('dark');
    });

    it('falls back to dark for unrecognized values', () => {
      window.localStorage.setItem(THEME_STORAGE_KEY, 'auto');
      expect(setupTheme()).toBe('dark');
      window.localStorage.setItem(THEME_STORAGE_KEY, 'garbage');
      expect(setupTheme()).toBe('dark');
    });
  });

  describe('useTheme', () => {
    it('toggleTheme flips DOM dataset and localStorage', () => {
      window.localStorage.setItem(THEME_STORAGE_KEY, 'dark');
      setupTheme();
      const { theme, toggleTheme } = useTheme();
      expect(theme.value).toBe('dark');

      toggleTheme();
      expect(theme.value).toBe('light');
      expect(document.documentElement.dataset.theme).toBe('light');
      expect(window.localStorage.getItem(THEME_STORAGE_KEY)).toBe('light');

      toggleTheme();
      expect(theme.value).toBe('dark');
      expect(document.documentElement.dataset.theme).toBe('dark');
      expect(window.localStorage.getItem(THEME_STORAGE_KEY)).toBe('dark');
    });
  });

  describe('appearance', () => {
    it.each([null, '', 'unknown', 'classic'])(
      'defaults to Classic for stored value %s',
      (stored) => {
        if (stored !== null) window.localStorage.setItem(APPEARANCE_STORAGE_KEY, stored);
        setupTheme();
        expect(useTheme().appearance.value).toBe('classic');
        expect(document.documentElement.dataset.appearance).toBe('classic');
      },
    );

    it('persists the style independently of light/dark mode and restores it on bootstrap', () => {
      window.localStorage.setItem(THEME_STORAGE_KEY, 'light');
      setupTheme();
      const { appearance, setAppearance, toggleTheme } = useTheme();
      setAppearance('swiss');
      expect(appearance.value).toBe('swiss');
      expect(document.documentElement.dataset.appearance).toBe('swiss');
      expect(document.documentElement.dataset.theme).toBe('light');
      expect(window.localStorage.getItem(APPEARANCE_STORAGE_KEY)).toBe('swiss');
      toggleTheme();
      setupTheme();
      expect(document.documentElement.dataset.appearance).toBe('swiss');
      expect(document.documentElement.dataset.theme).toBe('dark');
      setAppearance('classic');
      expect(window.localStorage.getItem(APPEARANCE_STORAGE_KEY)).toBe('classic');
    });

    it('keeps switching usable when localStorage is unavailable', () => {
      vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
        throw new Error('Storage unavailable');
      });
      vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
        throw new Error('Storage unavailable');
      });
      setupTheme();
      const { setAppearance } = useTheme();
      setAppearance('swiss');
      expect(document.documentElement.dataset.appearance).toBe('swiss');
      setAppearance('unsupported');
      expect(document.documentElement.dataset.appearance).toBe('classic');
    });
  });
});
