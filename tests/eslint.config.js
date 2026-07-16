import baseConfig from "../../eslint.config.js";
import globals from "globals";

export default [
  ...baseConfig,
  { languageOptions: { globals: { ...globals.node } } },
];
