import eslint from "@eslint/js";
import globals from "globals";
// The Rules of Hooks, enforced rather than remembered. `App.tsx` carried a
// comment recording a real violation and noting that this plugin was not
// installed -- a comment is not a control, and this codebase is hook-heavy.
import reactHooks from "eslint-plugin-react-hooks";
import tseslint from "typescript-eslint";

export default tseslint.config(
  {
    ignores: ["dist/**", "src-tauri/target/**"],
  },
  eslint.configs.recommended,
  ...tseslint.configs.recommended,
  {
    languageOptions: {
      globals: globals.browser,
    },
  },
  {
    files: ["src/**/*.{ts,tsx}"],
    plugins: { "react-hooks": reactHooks },
    rules: {
      // Errors, not warnings: `npm run lint` runs with `--max-warnings 0`, so a
      // warning would fail the gate anyway and would read as optional in the
      // editor. Both rules are the plugin's recommended pair.
      "react-hooks/rules-of-hooks": "error",
      "react-hooks/exhaustive-deps": "error",
    },
  },
);
