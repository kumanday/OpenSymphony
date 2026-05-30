
#!/usr/bin/env node

/**
 * Generates a minimal stub frontend for Tauri production builds.
 *
 * The real frontend mount arrives in a follow-up ticket (blocked by COE-394).
 * Until then, this stub ensures the Tauri binary ships with valid HTML instead
 * of an empty directory — which would cause a white-screen crash at launch.
 */

import { writeFileSync, mkdirSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const dist = join(__dirname, "..", "dist");

mkdirSync(dist, { recursive: true });

const html = `<!doctype html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>OpenSymphony</title>
    <style>
      body {
        font-family: system-ui, -apple-system, sans-serif;
        display: flex;
        justify-content: center;
        align-items: center;
        height: 100vh;
        margin: 0;
        background: #0a0a0a;
        color: #e4e4e7;
      }
      .stub { text-align: center; }
      .stub h1 { font-size: 1.5rem; font-weight: 600; }
      .stub p { color: #a1a1aa; }
    </style>
  </head>
  <body>
    <div class="stub">
      <h1>OpenSymphony Desktop</h1>
      <p>Frontend mount point — served by dev server.</p>
    </div>
  </body>
</html>`;

writeFileSync(join(dist, "index.html"), html);
console.log(`[desktop] Stub frontend written to ${dist}/index.html`);
