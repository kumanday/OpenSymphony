import { createConnection } from "node:net";
import { createServer, type Server, type ServerResponse } from "node:http";
import { existsSync, readFileSync, statSync } from "node:fs";
import { extname, isAbsolute, join, relative, resolve } from "node:path";
import {
  fixtureBundleList,
  fixtureGraphSnapshot,
} from "@opensymphony/graph";

const repoRoot = resolve(__dirname, "../../..");
const webDist = join(repoRoot, "apps/web/dist");

describe("Knowledge Graph renderer", () => {
  // Expects the repo test setup: `npm test` builds apps/web/dist before Jest,
  // and Playwright Chromium is installed in the CI/browser-test environment.
  it("renders a nonblank WebGL canvas in the built web app", async () => {
    const indexPath = join(webDist, "index.html");
    if (!existsSync(indexPath)) return;
    const playwright = await loadPlaywright();
    if (!playwright) return;
    const server = await startStaticServer(webDist);
    let browser: Awaited<ReturnType<typeof playwright.chromium.launch>> | null = null;
    try {
      try {
        browser = await playwright.chromium.launch({ headless: true });
      } catch (error) {
        if (String(error).includes("Executable doesn't exist")) return;
        throw error;
      }
      for (const viewport of [{ width: 1280, height: 800 }, { width: 390, height: 844 }]) {
        const page = await browser.newPage({ viewport });
        await page.goto(`${server.url}/app/`, { waitUntil: "domcontentloaded" });
        await page.getByRole("button", { name: "Knowledge Graph" }).click();
        await page.waitForFunction(() => {
          const canvas = document.querySelector("[data-testid='knowledge-graph-canvas']");
          return canvas instanceof HTMLCanvasElement && canvas.dataset.nonblank === "true";
        });
        const screenshot = await page.screenshot();
        expect(screenshot.length).toBeGreaterThan(1_000);
        const stats = await page.$eval(
          "[data-testid='knowledge-graph-canvas']",
          (canvasElement) => {
            const canvas = canvasElement as HTMLCanvasElement;
            const gl = canvas.getContext("webgl2") ?? canvas.getContext("webgl");
            if (!gl) return { changed: 0, total: 0, width: canvas.width, height: canvas.height };
            const width = canvas.width;
            const height = canvas.height;
            const pixels = new Uint8Array(width * height * 4);
            gl.readPixels(0, 0, width, height, gl.RGBA, gl.UNSIGNED_BYTE, pixels);
            const stride = Math.max(1, Math.floor((width * height) / 5_000));
            let changed = 0;
            let total = 0;
            for (let pixel = 0; pixel < width * height; pixel += stride) {
              const index = pixel * 4;
              if (pixels[index + 3] === 0) continue;
              total += 1;
              const delta = Math.abs(pixels[index] - 248)
                + Math.abs(pixels[index + 1] - 250)
                + Math.abs(pixels[index + 2] - 252);
              if (delta > 30) changed += 1;
            }
            return { changed, total, width, height };
          },
        );
        expect(stats.width).toBeGreaterThan(0);
        expect(stats.height).toBeGreaterThan(0);
        expect(stats.total).toBeGreaterThan(100);
        expect(stats.changed).toBeGreaterThan(20);
        await page.close();
      }
    } finally {
      await browser?.close();
      await new Promise<void>((resolveClose) => server.close(resolveClose));
    }
  }, 20_000);

  it("keeps the static test server contained to the web dist root", async () => {
    const server = await startStaticServer(webDist);
    try {
      const response = await rawHttpGet(server, "/app/%2e%2e%2f%2e%2e%2f%2e%2e%2fetc/passwd");
      expect(response.statusLine).toContain("404");
      expect(response.headers["x-static-containment"]).toBe("blocked");
    } finally {
      await new Promise<void>((resolveClose) => server.close(resolveClose));
    }
  });
});

async function loadPlaywright(): Promise<typeof import("playwright") | null> {
  try {
    return await import("playwright");
  } catch (error) {
    if (String(error).includes("Cannot find package") || String(error).includes("Cannot find module")) {
      return null;
    }
    throw error;
  }
}

function rawHttpGet(
  server: Server & { url: string },
  path: string,
): Promise<{ statusLine: string; headers: Record<string, string>; body: string }> {
  const address = server.address();
  if (!address || typeof address === "string") throw new Error("Unexpected static server address");
  return new Promise((resolveRequest, rejectRequest) => {
    const socket = createConnection(address.port, "127.0.0.1");
    let response = "";
    socket.setEncoding("utf8");
    socket.on("connect", () => {
      socket.write(`GET ${path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n`);
    });
    socket.on("data", (chunk) => {
      response += chunk;
    });
    socket.on("error", rejectRequest);
    socket.on("end", () => {
      const [head, body = ""] = response.split("\r\n\r\n");
      const [statusLine = "", ...headerLines] = head.split("\r\n");
      const headers = Object.fromEntries(headerLines.flatMap((line) => {
        const separator = line.indexOf(":");
        return separator > 0
          ? [[line.slice(0, separator).toLowerCase(), line.slice(separator + 1).trim()]]
          : [];
      }));
      resolveRequest({ statusLine, headers, body });
    });
  });
}

function startStaticServer(root: string): Promise<Server & { url: string }> {
  const rootPath = resolve(root);
  const server = createServer((request, response) => {
    const requestUrl = new URL(request.url ?? "/", "http://127.0.0.1");
    if (requestUrl.pathname === "/api/v1/memory/bundles") {
      writeJson(response, fixtureBundleList);
      return;
    }
    if (requestUrl.pathname === `/api/v1/memory/bundles/${fixtureGraphSnapshot.bundle_id}/graph`) {
      writeJson(response, fixtureGraphSnapshot);
      return;
    }
    const encodedPath = requestUrl.pathname === "/app/" || requestUrl.pathname === "/app"
      ? "index.html"
      : requestUrl.pathname.startsWith("/app/")
        ? requestUrl.pathname.slice("/app/".length)
        : "";
    let path = "";
    try {
      path = decodeURIComponent(encodedPath);
    } catch {
      response.writeHead(404, { "x-static-containment": "blocked" });
      response.end("not found");
      return;
    }
    if (!path || path.startsWith("/")) {
      response.writeHead(404);
      response.end("not found");
      return;
    }
    const filePath = resolve(rootPath, path);
    const relativePath = relative(rootPath, filePath);
    if (
      relativePath.startsWith("..")
      || isAbsolute(relativePath)
      || !existsSync(filePath)
      || !statSync(filePath).isFile()
    ) {
      response.writeHead(404, relativePath.startsWith("..") || isAbsolute(relativePath)
        ? { "x-static-containment": "blocked" }
        : undefined);
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

function writeJson(response: ServerResponse, body: unknown): void {
  response.writeHead(200, { "content-type": "application/json; charset=utf-8" });
  response.end(JSON.stringify(body));
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
