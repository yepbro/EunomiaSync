import js from '@eslint/js'
import eslintConfigPrettier from 'eslint-config-prettier'
import globals from 'globals'
import pluginVue from 'eslint-plugin-vue'
import tseslint from 'typescript-eslint'
import noRawUiText from './src/i18n/no-raw-ui-text.ts'

// The UI-020 gate: no user-visible raw text in Vue templates — every UI
// string goes through a vue-i18n key. The rule is written in-repo
// (eslint-plugin-vue has no no-raw-text rule; the @intlify plugin would be
// a new dependency outside the task's allow-list) and pinned by
// src/i18n/no-raw-ui-text.spec.ts. Imported with an explicit .ts
// extension: ESLint loads this config with Node, whose type stripping
// (enabled by default since Node 22.18) resolves it at lint time — CI's
// `node-version: 22` floats to a current 22.x, and vue-tsc checks the
// rule file itself (it is under src/); this config file belongs to no
// tsconfig project, so its import is unchecked wiring, which is fine.
const eunomiaPlugin = { rules: { 'no-raw-ui-text': noRawUiText } }

export default tseslint.config(
  {
    ignores: [
      'dist',
      'node_modules',
      'coverage',
      'playwright-report',
      'test-results',
      'src-tauri',
      'src/bindings.ts',
      // Cargo workspace target directory (root-level, holds build scripts output)
      'target',
    ],
  },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  ...pluginVue.configs['flat/recommended'],
  {
    files: ['**/*.{js,mjs,ts,vue}'],
    languageOptions: {
      globals: {
        ...globals.browser,
        ...globals.node,
      },
    },
  },
  {
    files: ['**/*.vue'],
    languageOptions: {
      parserOptions: {
        parser: tseslint.parser,
      },
    },
  },
  {
    // Ambient module shims in declaration files legitimately use `{}`/`any`.
    files: ['**/*.d.ts'],
    rules: {
      '@typescript-eslint/no-empty-object-type': 'off',
      '@typescript-eslint/no-explicit-any': 'off',
    },
  },
  {
    // UI-020: templates carry no user-visible raw text — vue-i18n keys only.
    files: ['**/*.vue'],
    name: 'eunomia/i18n',
    plugins: { eunomia: eunomiaPlugin },
    rules: {
      'eunomia/no-raw-ui-text': 'error',
    },
  },
  eslintConfigPrettier,
)
