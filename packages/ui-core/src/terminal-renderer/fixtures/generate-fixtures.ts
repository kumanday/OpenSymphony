/**
 * Generate JSON fixture files for terminal renderer testing.
 *
 * Run with: npx tsx generate-fixtures.ts
 */

import { writeFileSync, mkdirSync, existsSync } from "node:fs";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import {
  generateBurstFrames,
  generateRealisticSession,
  generateBurstySession,
  exportFixturesToJson,
} from "./terminal-fixtures.js";

const __filename = fileURLToPath(import.meta.url);
const fixturesDir = resolve(dirname(__filename), "json");

if (!existsSync(fixturesDir)) {
  mkdirSync(fixturesDir, { recursive: true });
}

// 1. Small burst fixture (100 frames)
console.log("Generating small burst fixture...");
const smallBurst = generateBurstFrames(100);
writeFileSync(
  resolve(fixturesDir, "burst-small.json"),
  exportFixturesToJson(smallBurst),
);

// 2. Medium burst fixture (1000 frames)
console.log("Generating medium burst fixture...");
const mediumBurst = generateBurstFrames(1000, { includeAnsiCodes: true });
writeFileSync(
  resolve(fixturesDir, "burst-medium.json"),
  exportFixturesToJson(mediumBurst),
);

// 3. Large burst fixture (5000 frames)
console.log("Generating large burst fixture...");
const largeBurst = generateBurstFrames(5000, { includeAnsiCodes: true });
writeFileSync(
  resolve(fixturesDir, "burst-large.json"),
  exportFixturesToJson(largeBurst),
);

// 4. Realistic session fixture (30 seconds @ 30fps)
console.log("Generating realistic session fixture...");
const realisticSession = generateRealisticSession(30000, 30);
writeFileSync(
  resolve(fixturesDir, "session-realistic.json"),
  exportFixturesToJson(realisticSession),
);

// 5. Bursty session fixture
console.log("Generating bursty session fixture...");
const burstySession = generateBurstySession(30000, 2000, 100, 5);
writeFileSync(
  resolve(fixturesDir, "session-bursty.json"),
  exportFixturesToJson(burstySession),
);

console.log("All fixtures generated successfully!");
console.log(`Output directory: ${fixturesDir}`);
