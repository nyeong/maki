export default [
  {
    files: ["assets/**/*.js"],
    languageOptions: {
      ecmaVersion: 2024,
      sourceType: "script",
      globals: {
        document: "readonly",
        encodeURIComponent: "readonly",
        fetch: "readonly",
        location: "readonly",
        URLSearchParams: "readonly",
      },
    },
    rules: {
      curly: ["error", "multi-line"],
      eqeqeq: ["error", "always"],
      "no-constant-condition": "error",
      "no-undef": "error",
      "no-unreachable": "error",
      "no-unused-vars": "error",
      "prefer-const": "error",
    },
  },
];
