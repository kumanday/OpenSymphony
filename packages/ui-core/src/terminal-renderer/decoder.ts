/**
 * Terminal frame decoder.
 *
 * Provides pure decoding helpers for TerminalFrame payloads. A future worker
 * wrapper can call these helpers without changing the renderer API.
 */

import type { TerminalFrame, TerminalEncoding } from "@opensymphony/gateway-schema";

// -- ANSI escape code handling --

const ANSI_ESCAPE = /\x1b\[[0-9;]*[a-zA-Z]/g;

/**
 * Strip ANSI escape sequences from a text payload.
 * Returns the cleaned text plus an array of style spans.
 */
export function decodeAnsiText(raw: string): { text: string; spans: StyleSpan[] } {
  const spans: StyleSpan[] = [];
  let text = "";
  let lastIndex = 0;
  let match: RegExpExecArray | null;

  const regex = new RegExp(ANSI_ESCAPE.source, "g");
  while ((match = regex.exec(raw)) !== null) {
    // Push plain text before this escape sequence
    text += raw.slice(lastIndex, match.index);
    lastIndex = match.index + match[0].length;

    // Parse the escape sequence for style information
    const code = match[0];
    if (code.startsWith("\x1b[")) {
      const params = code.slice(2, -1).split(";").map(Number);
      spans.push({
        start: text.length,
        styles: parseAnsiCodes(params),
      });
    }
  }

  // Push remaining text
  text += raw.slice(lastIndex);

  return { text, spans };
}

export interface StyleSpan {
  start: number;
  styles: TextStyle[];
}

export type TextStyle = "bold" | "dim" | "italic" | "underline" | "strikethrough" | ColorStyle;

export interface ColorStyle {
  type: "foreground" | "background";
  color: string; // CSS color string
}

/** Parse ANSI SGR codes into style descriptors. */
function parseAnsiCodes(codes: number[]): TextStyle[] {
  const styles: TextStyle[] = [];

  for (const code of codes) {
    switch (code) {
      case 0: // Reset - we handle this by clearing styles in the renderer
        break;
      case 1:
        styles.push("bold");
        break;
      case 2:
        styles.push("dim");
        break;
      case 3:
        styles.push("italic");
        break;
      case 4:
        styles.push("underline");
        break;
      case 9:
        styles.push("strikethrough");
        break;
      case 30:
      case 31:
      case 32:
      case 33:
      case 34:
      case 35:
      case 36:
      case 37:
        styles.push({ type: "foreground", color: ansiColorToHex(code) });
        break;
      case 40:
      case 41:
      case 42:
      case 43:
      case 44:
      case 45:
      case 46:
      case 47:
        styles.push({ type: "background", color: ansiColorToHex(code - 10) });
        break;
      case 90:
      case 91:
      case 92:
      case 93:
      case 94:
      case 95:
      case 96:
      case 97:
        styles.push({ type: "foreground", color: ansiBrightColorToHex(code) });
        break;
      case 100:
      case 101:
      case 102:
      case 103:
      case 104:
      case 105:
      case 106:
      case 107:
        styles.push({ type: "background", color: ansiBrightColorToHex(code) });
        break;
      default:
        break;
    }
  }

  return styles;
}

/** Map standard ANSI colors to hex. */
function ansiColorToHex(code: number): string {
  const colors: Record<number, string> = {
    30: "#000000", // black
    31: "#cd3131", // red
    32: "#0dbc79", // green
    33: "#e5e510", // yellow
    34: "#2472c8", // blue
    35: "#bc3fbc", // magenta
    36: "#11a8cd", // cyan
    37: "#e5e5e5", // white
  };
  return colors[code] || "#e5e5e5";
}

/** Map bright ANSI colors to hex. */
function ansiBrightColorToHex(code: number): string {
  const colors: Record<number, string> = {
    90: "#666666", // bright black (gray)
    91: "#f14c4c", // bright red
    92: "#23d18b", // bright green
    93: "#f5f543", // bright yellow
    94: "#3b8eea", // bright blue
    95: "#d670d6", // bright magenta
    96: "#29b8db", // bright cyan
    97: "#ffffff", // bright white
    100: "#666666", // bright bg black
    101: "#f14c4c", // bright bg red
    102: "#23d18b", // bright bg green
    103: "#f5f543", // bright bg yellow
    104: "#3b8eea", // bright bg blue
    105: "#d670d6", // bright bg magenta
    106: "#29b8db", // bright bg cyan
    107: "#ffffff", // bright bg white
  };
  return colors[code] || "#ffffff";
}

// -- Message protocol for worker communication --

export type DecoderRequest =
  | { type: "decode"; payload: string; encoding: TerminalEncoding; frame: TerminalFrame }
  | { type: "decodeBatch"; payloads: { content: string; encoding: TerminalEncoding; frame: TerminalFrame }[] };

export interface DecodedFrame {
  frame: TerminalFrame;
  text: string;
  spans: StyleSpan[];
  decodedAt: number;
}

export type DecoderResponse =
  | { type: "decoded"; result: DecodedFrame }
  | { type: "decodedBatch"; results: DecodedFrame[] }
  | { type: "error"; error: string };

/** Decode a single frame payload. */
export function decodeFrame(
  content: string,
  encoding: TerminalEncoding,
  frame: TerminalFrame,
): DecodedFrame {
  let text = content;

  // Decode base64 content if needed
  if (encoding === "base64") {
    try {
      text = atob(content);
    } catch {
      text = `[decode error: invalid base64]`;
    }
  }

  const { text: cleanedText, spans } = decodeAnsiText(text);

  return {
    frame,
    text: cleanedText,
    spans,
    decodedAt: performance.now(),
  };
}

/** Decode a batch of frame payloads efficiently. */
export function decodeBatch(
  payloads: { content: string; encoding: TerminalEncoding; frame: TerminalFrame }[],
): DecodedFrame[] {
  return payloads.map((p) => decodeFrame(p.content, p.encoding, p.frame));
}
