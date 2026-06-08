import * as fs from "fs";
import * as path from "path";

const DIST_DIR = path.resolve(__dirname, "../dist");

describe("web build output", () => {
  beforeAll(() => {
    expect(fs.existsSync(DIST_DIR)).toBe(true);
  });

  it("should contain index.html", () => {
    const indexPath = path.join(DIST_DIR, "index.html");
    expect(fs.existsSync(indexPath)).toBe(true);
    const content = fs.readFileSync(indexPath, "utf-8");
    expect(content).toContain("<!doctype html>");
    expect(content).toContain('<div id="root"></div>');
  });

  it("should reference a cache-busted JS bundle", () => {
    const content = fs.readFileSync(
      path.join(DIST_DIR, "index.html"),
      "utf-8",
    );
    expect(content).toMatch(/src="\/app\/assets\/main-[a-zA-Z0-9_-]+.js"/);
  });

  it("should contain an assets directory with a JS bundle", () => {
    const assetsDir = path.join(DIST_DIR, "assets");
    expect(fs.existsSync(assetsDir)).toBe(true);
    const files = fs.readdirSync(assetsDir);
    const jsFiles = files.filter((f) => f.endsWith(".js"));
    expect(jsFiles.length).toBeGreaterThan(0);
  });

  it("should mount the shared OpenSymphony app shell", () => {
    const assetsDir = path.join(DIST_DIR, "assets");
    const files = fs.readdirSync(assetsDir).filter((file) => file.endsWith(".js"));
    const bundle = files
      .map((file) => fs.readFileSync(path.join(assetsDir, file), "utf-8"))
      .join("\n");

    expect(bundle).toContain("data-opensymphony-app-shell");
    expect(bundle).not.toMatch(/Terminal Renderer Demo/);
  });

  it("should not include Tauri or desktop-only references", () => {
    const assetsDir = path.join(DIST_DIR, "assets");
    const files = fs.readdirSync(assetsDir);
    for (const file of files) {
      if (file.endsWith(".js")) {
        const content = fs.readFileSync(path.join(assetsDir, file), "utf-8");
        expect(content).not.toMatch(/tauri/i);
        expect(content).not.toMatch(/desktop-transport/i);
      }
    }
  });
});
