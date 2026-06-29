import { createServer, type Server } from "node:http";
import { existsSync, readFileSync, statSync } from "node:fs";
import { extname, join, resolve } from "node:path";
import { chromium } from "playwright";

const repoRoot = resolve(__dirname, "../../..");
const webDist = join(repoRoot, "apps/web/dist");

describe("Knowledge Graph renderer", () => {
  it("renders a nonblank WebGL canvas in the built web app", async () => {
    const indexPath = join(webDist, "index.html");
    if (!existsSync(indexPath)) {
      console.warn("Skipping built-web WebGL proof because apps/web/dist is missing.");
      return;
    }
    const server = await startStaticServer(webDist);
    const browser = await chromium.launch({ headless: true });
    try {
      const page = await browser.newPage({ viewport: { width: 1024, height: 768 } });
      await page.goto(`${server.url}/app/`, { waitUntil: "domcontentloaded" });
      await page.getByRole("button", { name: "Knowledge Graph" }).click();
      await page.waitForSelector("[data-testid='knowledge-graph-canvas'][data-nonblank='true']");
      await page.waitForTimeout(300);
      const dataUrl = await page.$eval(
        "[data-testid='knowledge-graph-canvas']",
        (canvas) => (canvas as HTMLCanvasElement).toDataURL("image/png"),
      );
      expect(dataUrl).toMatch(/^data:image\/png;base64,/);
      expect(dataUrl.length).toBeGreaterThan(1_000);
    } finally {
      await browser.close();
      await new Promise<void>((resolveClose) => server.close(resolveClose));
    }
  }, 20_000);
});

function startStaticServer(root: string): Promise<Server & { url: string }> {
  const server = createServer((request, response) => {
    const requestUrl = new URL(request.url ?? "/", "http://127.0.0.1");
    const path = requestUrl.pathname === "/app/" || requestUrl.pathname === "/app"
      ? "index.html"
      : requestUrl.pathname.replace(/^\/app\//, "");
    const filePath = resolve(root, path);
    if (!filePath.startsWith(root) || !existsSync(filePath) || !statSync(filePath).isFile()) {
      response.writeHead(404);
      response.end("not found");
      return;
    }
    response.writeHead(200, { "content-type": contentType(filePath) });
    response.end(readFileSync(filePath));
  }) as Server & { url: string };
  return new Promise((resolveListen) => {
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      if (!address || typeof address === "string") throw new Error("Unexpected static server address");
      server.url = `http://127.0.0.1:${address.port}`;
      resolveListen(server);
    });
  });
}

function contentType(filePath: string): string {
  switch (extname(filePath)) {
    case ".html":
      return "text/html; charset=utf-8";
    case ".js":
      return "text/javascript; charset=utf-8";
    case ".css":
      return "text/css; charset=utf-8";
    case ".png":
      return "image/png";
    default:
      return "application/octet-stream";
  }
}
