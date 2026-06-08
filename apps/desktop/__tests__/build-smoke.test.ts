import * as fs from "fs";
import * as path from "path";

const DIST_DIR = path.resolve(__dirname, "../dist");

describe("desktop build output", () => {
  beforeAll(() => {
    expect(fs.existsSync(DIST_DIR)).toBe(true);
  });

  it("contains a Vite-built index.html entrypoint", () => {
    const indexPath = path.join(DIST_DIR, "index.html");
    expect(fs.existsSync(indexPath)).toBe(true);
    const content = fs.readFileSync(indexPath, "utf-8");
    expect(content).toContain("<!doctype html>");
    expect(content).toContain('<div id="root"></div>');
    expect(content).toMatch(/assets\/main-[a-zA-Z0-9_-]+.js/);
  });

  it("mounts the shared app shell instead of the old stub page", () => {
    const assetsDir = path.join(DIST_DIR, "assets");
    const files = fs.readdirSync(assetsDir).filter((file) => file.endsWith(".js"));
    const bundle = files
      .map((file) => fs.readFileSync(path.join(assetsDir, file), "utf-8"))
      .join("\n");

    expect(bundle).toContain("data-opensymphony-app-shell");
    expect(bundle).toContain("OpenSymphony Desktop");
    expect(bundle).not.toMatch(/Tauri transport adapter not yet implemented/);
    expect(bundle).not.toMatch(/stub frontend/i);
  });
});
