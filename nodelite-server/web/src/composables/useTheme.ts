import { ref, type Ref } from 'vue';

export type Theme = 'light' | 'dark';
export type Appearance = 'classic' | 'swiss';

export const THEME_STORAGE_KEY = 'nodelite.ui.theme';
export const APPEARANCE_STORAGE_KEY = 'nodelite.ui.appearance';
const THEME_SWITCHING_CLASS = 'theme-switching';

function readStoredTheme(): Theme {
  try {
    const stored = window.localStorage.getItem(THEME_STORAGE_KEY);
    return stored === 'light' ? 'light' : 'dark';
  } catch {
    return 'dark';
  }
}

function readStoredAppearance(): Appearance {
  try {
    return window.localStorage.getItem(APPEARANCE_STORAGE_KEY) === 'swiss' ? 'swiss' : 'classic';
  } catch {
    return 'classic';
  }
}

function runAfterPaint(callback: () => void): void {
  if (typeof window.requestAnimationFrame === 'function') {
    window.requestAnimationFrame(() => window.requestAnimationFrame(callback));
    return;
  }
  window.setTimeout(callback, 0);
}

function suppressThemeTransitions(): void {
  const root = document.documentElement;
  root.classList.add(THEME_SWITCHING_CLASS);
  runAfterPaint(() => root.classList.remove(THEME_SWITCHING_CLASS));
}

function writeTheme(theme: Theme, suppressTransitions = false): void {
  if (suppressTransitions) suppressThemeTransitions();
  document.documentElement.dataset.theme = theme;
  try {
    window.localStorage.setItem(THEME_STORAGE_KEY, theme);
  } catch {
    // localStorage unavailable (private mode, quota) — DOM still gets the attr
  }
}

let themeRef: Ref<Theme> | null = null;
const appearanceRef = ref<Appearance>(readStoredAppearance());

function ensureRef(): Ref<Theme> {
  if (themeRef === null) {
    themeRef = ref<Theme>(readStoredTheme());
  }
  return themeRef;
}

/** The inline bootstrap in index.html also restores these values to avoid a first-paint flash. */
export function setupTheme(): Theme {
  const theme = readStoredTheme();
  document.documentElement.classList.remove(THEME_SWITCHING_CLASS);
  document.documentElement.dataset.theme = theme;
  appearanceRef.value = readStoredAppearance();
  document.documentElement.dataset.appearance = appearanceRef.value;
  ensureRef().value = theme;
  return theme;
}

export function useTheme(): {
  theme: Ref<Theme>;
  appearance: Ref<Appearance>;
  toggleTheme: () => void;
  setAppearance: (value: string) => void;
} {
  const theme = ensureRef();

  function toggleTheme(): void {
    const next: Theme = theme.value === 'light' ? 'dark' : 'light';
    theme.value = next;
    writeTheme(next, true);
  }

  function setAppearance(value: string): void {
    const next: Appearance = value === 'swiss' ? 'swiss' : 'classic';
    suppressThemeTransitions();
    appearanceRef.value = next;
    document.documentElement.dataset.appearance = next;
    try {
      window.localStorage.setItem(APPEARANCE_STORAGE_KEY, next);
    } catch {
      // The selection still works when browser storage is unavailable.
    }
  }

  return { theme, appearance: appearanceRef, toggleTheme, setAppearance };
}
