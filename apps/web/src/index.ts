import { HttpGatewayTransport } from "@opensymphony/api-client";
import type { GatewayTransport } from "@opensymphony/api-client";

export interface BrowserTransportAdapter extends GatewayTransport {
  connect(token?: string): Promise<void>;
}

export function createWebTransport(baseUri = ""): BrowserTransportAdapter {
  const transport = new HttpGatewayTransport({
    baseUri,
    transport: "loopback_http",
  }) as HttpGatewayTransport & { connect(token?: string): Promise<void> };
  transport.connect = async () => undefined;
  return transport;
}
