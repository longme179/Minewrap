// ESLint flat config — warn-level lint for the UI (svelte-check owns
// typechecking; ESLint flags unused vars and obvious mistakes).
import { parser as tsParser } from "typescript-eslint";

export default [
  {
    files: ["src/**/*.ts", "scripts/**/*.ts"],
    languageOptions: {
      parser: tsParser,
      ecmaVersion: "latest",
      sourceType: "module",
    },
    rules: {
      "no-unused-vars": ["warn", { argsIgnorePattern: "^_" }],
      "no-undef": "off",
    },
  },
  {
    ignores: ["node_modules", "dist"],
  },
];
