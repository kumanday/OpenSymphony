/** @type {import('jest').Config} */
export default {
  rootDir: ".",
  testEnvironment: "node",
  transform: {
    "^.+\\.tsx?$": [
      "ts-jest",
      {
        useESM: true,
      },
    ],
  },
  extensionsToTreatAsEsm: [".ts"],
  moduleNameMapper: {
    "^(\\.{1,2}/.*)\\.js$": "$1",
    "^@opensymphony/(.+)$": "<rootDir>/packages/$1/src/index.ts",
  },
  testMatch: ["**/__tests__/**/*.test.ts"],
  transformIgnorePatterns: ["/node_modules/"],
};