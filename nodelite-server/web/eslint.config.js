import pluginVue from 'eslint-plugin-vue';
import tseslint from 'typescript-eslint';
import vueParser from 'vue-eslint-parser';
import skipFormatting from '@vue/eslint-config-prettier/skip-formatting';

export default [
  {
    name: 'app/files-to-lint',
    files: ['**/*.{ts,mts,tsx,vue}'],
  },
  {
    name: 'app/files-to-ignore',
    ignores: ['**/dist/**', '**/dist-ssr/**', '**/coverage/**', '**/playwright-report/**', '**/test-results/**'],
  },
  ...pluginVue.configs['flat/recommended'],
  ...tseslint.configs.recommended,
  {
    files: ['**/*.vue'],
    languageOptions: {
      parser: vueParser,
      parserOptions: {
        parser: tseslint.parser,
        ecmaVersion: 2024,
        extraFileExtensions: ['.vue'],
      },
    },
    rules: {
      ...tseslint.configs.eslintRecommended.rules,
      'vue/block-lang': ['error', { script: { lang: ['ts'], allowNoLang: false } }],
    },
  },
  skipFormatting,
  {
    rules: {
      'vue/multi-word-component-names': 'off',
    },
  },
];
