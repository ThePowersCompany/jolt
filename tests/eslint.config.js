import tseslint from "typescript-eslint";
import parser from "@typescript-eslint/parser";
import globals from "globals";

export default [
  {
    name: "ESLint Config",
    languageOptions: {
      parser,
      parserOptions: {
        ecmaVersion: "latest",
        sourceType: "module",
        ecmaFeatures: {
          jsx: true,
        },
      },
      globals: { ...globals.node },
    },
    plugins: {},
    files: ["**/*.{js,mjs,cjs,ts,jsx,tsx}"],
    rules: {},
  },
  { files: ["**/*.{js,mjs,cjs,ts,jsx,tsx}"] },
  ...tseslint.configs.recommended,
  {
    // Custom typescript-eslint rules ripped from the next.js repo eslint config
    rules: {
      "@typescript-eslint/array-type": "off",
      "@typescript-eslint/ban-ts-comment": "off",
      "@typescript-eslint/ban-tslint-comment": "off",
      "@typescript-eslint/no-empty-object-type": "off",
      "@typescript-eslint/no-restricted-types": "off",
      "@typescript-eslint/no-unsafe-function-type": "off",
      "@typescript-eslint/no-wrapper-object-types": "off",
      "@typescript-eslint/class-literal-property-style": "off",
      "@typescript-eslint/consistent-generic-constructors": "off",
      "@typescript-eslint/consistent-indexed-object-style": "off",
      "@typescript-eslint/consistent-type-definitions": "off",
      "@typescript-eslint/no-empty-function": "off",
      "@typescript-eslint/no-namespace": "off",
      "@typescript-eslint/no-shadow": "off",
      "@typescript-eslint/no-empty-interface": "off",
      "@typescript-eslint/no-explicit-any": "off",
      "@typescript-eslint/no-inferrable-types": "off",
      "@typescript-eslint/no-require-imports": "off",
      "@typescript-eslint/no-var-requires": "off",
      "@typescript-eslint/prefer-for-of": "off",
      "@typescript-eslint/prefer-function-type": "off",
      "@typescript-eslint/no-this-alias": "off",
      "@typescript-eslint/triple-slash-reference": "off",
      "no-var": "off",
      "prefer-const": "off",
      "prefer-rest-params": "off",
      "prefer-spread": "off",
      "no-unused-expressions": "off",
      "@typescript-eslint/no-unused-expressions": [
        "error",
        {
          allowShortCircuit: true,
          allowTernary: true,
          allowTaggedTemplates: true,
        },
      ],
      "no-unused-vars": "off",
      "@typescript-eslint/no-unused-vars": "off",
      "no-use-before-define": "off",
      "no-useless-constructor": "off",
      "@typescript-eslint/no-use-before-define": "off",
      "@typescript-eslint/no-useless-constructor": "error",
      "@typescript-eslint/prefer-literal-enum-member": "error",
    },
  },
];
