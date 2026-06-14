/**
 * DOM-based terminal viewer component.
 *
 * Renders decoded frames to the browser DOM with support for:
 * - Scrollback with virtualized buffering
 * - Text search and highlighting
 * - Copy to clipboard
 * - Jump to latest output
 * - ANSI color display
 */

import { searchText } from "@opensymphony/ui-core";
import type { DecodedFrame, ScrollbackBuffer, TextStyle, ColorStyle } from "@opensymphony/ui-core";
import type { TerminalRenderer } from "@opensymphony/ui-core";
import type { TerminalLogAssociation } from "@opensymphony/gateway-schema";

export interface TerminalViewerConfig {
  fontFamily: string;
  fontSize: number;
  lineHeight: number;
  wrapLines: boolean;
  maxVisibleFrames: number;
}

export interface TerminalViewerOptions {
  container: HTMLElement;
  config?: Partial<TerminalViewerConfig>;
}

/**
 * Terminal viewer that renders frames to a DOM container.
 *
 * Maintains live DOM element references to avoid O(n²) thrashing:
 * - New lines are appended directly (not cloned)
 * - Old lines are removed when exceeding maxVisibleFrames
 * - Search operates on in-document elements
 */
export class TerminalViewer {
  private container: HTMLElement;
  private config: TerminalViewerConfig;
  private renderer: TerminalRenderer | null;
  private scrollContainer?: HTMLElement;
  private toolbar?: HTMLElement;
  private searchInput?: HTMLInputElement;
  private searchButton?: HTMLButtonElement;
  private copyButton?: HTMLButtonElement;
  private jumpButton?: HTMLButtonElement;
  private statusSpan?: HTMLSpanElement;
  private detachRender?: () => void;
  /** Live DOM elements that are children of scrollContainer. */
  private lineElements: HTMLElement[] = [];
  private searchTerm = "";
  private searchResults: number[] = [];
  private currentSearchIndex = 0;
  private pendingFocusFrameIndex: number | undefined;
  private associationInfo: TerminalLogAssociation | undefined;

  constructor(renderer: TerminalRenderer, options: TerminalViewerOptions) {
    this.renderer = renderer;
    this.container = options.container;
    this.config = {
      fontFamily: options.config?.fontFamily ?? "Menlo, Monaco, 'Courier New', monospace",
      fontSize: options.config?.fontSize ?? 14,
      lineHeight: options.config?.lineHeight ?? 1.4,
      wrapLines: options.config?.wrapLines ?? true,
      maxVisibleFrames: options.config?.maxVisibleFrames ?? 200,
    };

    // Build UI structure
    this.buildUI();
    this.attachRenderer();
  }

  /**
   * Build the terminal viewer UI.
   */
  private buildUI(): void {
    // Create terminal container
    this.container.innerHTML = "";
    this.container.style.cssText = `
      display: flex;
      flex-direction: column;
      height: 100%;
      max-height: 600px;
      background: #0d1117;
      border: 1px solid #30363d;
      border-radius: 6px;
      overflow: hidden;
      font-family: ${this.config.fontFamily};
    `;

    // Toolbar
    const toolbar = document.createElement("div");
    toolbar.className = "terminal-toolbar";
    toolbar.style.cssText = `
      display: flex;
      gap: 8px;
      padding: 8px;
      background: #161b22;
      border-bottom: 1px solid #30363d;
      align-items: center;
    `;

    // Search input
    const searchInput = document.createElement("input");
    searchInput.type = "text";
    searchInput.placeholder = "Search terminal output...";
    searchInput.style.cssText = `
      flex: 1;
      padding: 4px 8px;
      background: #0d1117;
      border: 1px solid #30363d;
      border-radius: 4px;
      color: #c9d1d9;
      font-size: 13px;
    `;

    // Search button
    const searchButton = document.createElement("button");
    searchButton.textContent = "Search";
    searchButton.style.cssText = `
      padding: 4px 12px;
      background: #21262d;
      border: 1px solid #30363d;
      border-radius: 4px;
      color: #c9d1d9;
      cursor: pointer;
      font-size: 13px;
    `;

    // Copy button
    const copyButton = document.createElement("button");
    copyButton.textContent = "Copy";
    copyButton.style.cssText = `
      padding: 4px 12px;
      background: #21262d;
      border: 1px solid #30363d;
      border-radius: 4px;
      color: #c9d1d9;
      cursor: pointer;
      font-size: 13px;
    `;

    // Jump to latest button
    const jumpButton = document.createElement("button");
    jumpButton.textContent = "Latest";
    jumpButton.style.cssText = `
      padding: 4px 12px;
      background: #238636;
      border: 1px solid #2ea043;
      border-radius: 4px;
      color: #ffffff;
      cursor: pointer;
      font-size: 13px;
    `;

    // Status span
    const statusSpan = document.createElement("span");
    statusSpan.className = "terminal-status-bar";
    statusSpan.style.cssText = `
      margin-left: auto;
      color: #8b949e;
      font-size: 12px;
    `;

    // Add toolbar elements
    toolbar.appendChild(searchInput);
    toolbar.appendChild(searchButton);
    toolbar.appendChild(copyButton);
    toolbar.appendChild(jumpButton);
    toolbar.appendChild(statusSpan);
    this.container.appendChild(toolbar);

    // Scroll container for terminal output
    const scrollContainer = document.createElement("div");
    scrollContainer.className = "terminal-scroll-container";
    scrollContainer.style.cssText = `
      flex: 1;
      overflow-y: auto;
      padding: 8px;
      font-size: ${this.config.fontSize}px;
      line-height: ${this.config.lineHeight};
      word-wrap: ${this.config.wrapLines ? "break-word" : "normal"};
    `;
    this.container.appendChild(scrollContainer);

    // Bound handlers for cleanup in dispose()
    this._onSearch = () => this.performSearch();
    this._onSearchKeypress = (e: KeyboardEvent) => {
      if (e.key === "Enter") this.performSearch();
    };
    this._onCopy = () => this.copyToClipboard();
    this._onJump = () => this.jumpToLatest();
    this._onScroll = () => this.syncScrollStateFromDom();

    searchButton.addEventListener("click", this._onSearch);
    searchInput.addEventListener("keypress", this._onSearchKeypress);
    copyButton.addEventListener("click", this._onCopy);
    jumpButton.addEventListener("click", this._onJump);
    scrollContainer.addEventListener("scroll", this._onScroll, { passive: true });

    this.toolbar = toolbar;
    this.searchInput = searchInput;
    this.searchButton = searchButton;
    this.copyButton = copyButton;
    this.jumpButton = jumpButton;
    this.statusSpan = statusSpan;
    this.scrollContainer = scrollContainer;
  }

  /** Bound handlers for removeEventListener in dispose() */
  private _onSearch?: () => void;
  private _onSearchKeypress?: (e: KeyboardEvent) => void;
  private _onCopy?: () => void;
  private _onJump?: () => void;
  private _onScroll?: () => void;

  /**
   * Attach to the renderer and listen for updates.
   */
  private attachRenderer(): void {
    const renderer = this.renderer;
    if (!renderer) return;

    this.detachRender = renderer.onRender((decodedFrames: DecodedFrame[], buffer: ScrollbackBuffer) => {
      this.renderFrames(decodedFrames, buffer);
    });
  }

  /**
   * Render new frames to the DOM using incremental updates.
   * Only appends new lines and removes pruned ones to avoid O(n²) DOM thrashing.
   * When called with empty decodedFrames (e.g., from scrollToFrame/jumpToLatest),
   * rebuilds the visible DOM from the buffer's current visibleFrames.
   */
  private renderFrames(decodedFrames: DecodedFrame[], buffer: ScrollbackBuffer): void {
    const scrollContainer = this.scrollContainer;
    if (!scrollContainer) return;

    if (decodedFrames.length > 0 && buffer.atBottom) {
      // Append only new frames
      const firstFrameIndex = buffer.totalFrames - decodedFrames.length;
      for (let i = 0; i < decodedFrames.length; i++) {
        const frame = decodedFrames[i];
        const lineElement = this.createLineElement(frame, firstFrameIndex + i);
        this.lineElements.push(lineElement);
        scrollContainer.appendChild(lineElement);
      }

      // Enforce max visible lines by removing oldest from DOM
      const excess = this.lineElements.length - this.config.maxVisibleFrames;
      for (let i = 0; i < excess; i++) {
        const removed = this.lineElements.shift();
        if (removed && removed.parentNode === scrollContainer) {
          scrollContainer.removeChild(removed);
        }
      }
    } else if (decodedFrames.length === 0 || this.visibleDomWasPruned(buffer)) {
      // Rebuild visible DOM from buffer (for scrollToFrame/jumpToLatest/history view)
      this.rebuildVisibleFrames(buffer);
    }

    this.updateStatus(buffer);

    // Auto-scroll to bottom if at latest
    if (buffer.atBottom) {
      this.scrollToBottom();
    }
  }

  /**
   * Detect when the currently rendered history has fallen out of retained buffer history.
   */
  private visibleDomWasPruned(buffer: ScrollbackBuffer): boolean {
    const firstLine = this.lineElements[0];
    if (!firstLine) return true;

    const firstIndex = Number(firstLine.dataset.lineIndex);
    return !Number.isFinite(firstIndex) || firstIndex < buffer.offset;
  }

  /**
   * Rebuild the DOM from the current visible frame window.
   */
  private rebuildVisibleFrames(buffer: ScrollbackBuffer): void {
    const scrollContainer = this.scrollContainer;
    if (!scrollContainer) return;

    scrollContainer.innerHTML = "";
    this.lineElements = [];

    const window = this.getRenderableWindow(buffer);
    for (let i = 0; i < window.frames.length; i++) {
      const frame = window.frames[i];
      const lineElement = this.createLineElement(frame, window.offset + i);
      this.lineElements.push(lineElement);
      scrollContainer.appendChild(lineElement);
    }
  }

  /**
   * Pick the DOM-sized slice to render from the buffer window.
   */
  private getRenderableWindow(buffer: ScrollbackBuffer): { frames: DecodedFrame[]; offset: number } {
    const focusIndex = this.pendingFocusFrameIndex;
    this.pendingFocusFrameIndex = undefined;

    if (buffer.visibleFrames.length <= this.config.maxVisibleFrames) {
      return { frames: buffer.visibleFrames, offset: buffer.offset };
    }

    const max = this.config.maxVisibleFrames;
    let start = 0;
    const windowEnd = buffer.offset + buffer.visibleFrames.length;
    if (focusIndex !== undefined && focusIndex >= buffer.offset && focusIndex < windowEnd) {
      const focusOffset = focusIndex - buffer.offset;
      start = Math.max(0, Math.min(focusOffset - Math.floor(max / 2), buffer.visibleFrames.length - max));
    } else if (buffer.atBottom) {
      start = buffer.visibleFrames.length - max;
    } else {
      // Center on the middle of the visible window when not at bottom
      start = Math.max(0, Math.min(
        Math.floor(buffer.visibleFrames.length / 2) - Math.floor(max / 2),
        buffer.visibleFrames.length - max,
      ));
    }
    return {
      frames: buffer.visibleFrames.slice(start, start + max),
      offset: buffer.offset + start,
    };
  }

  /**
   * Create a DOM element for a decoded frame line.
   */
  private createLineElement(frame: DecodedFrame, lineIndex: number): HTMLElement {
    const line = document.createElement("div");
    line.style.cssText = `
      padding: 2px 0;
      white-space: pre-wrap;
    `;
    line.dataset.lineIndex = String(lineIndex);
    if (frame.frame.source_event_id) {
      line.dataset.eventId = frame.frame.source_event_id;
    }
    if (frame.frame.frame_sequence !== undefined) {
      line.dataset.frameSequence = String(frame.frame.frame_sequence);
    }

    // Apply ANSI color styling
    if (frame.spans.length > 0) {
      this.applySpanStyles(line, frame.text, frame.spans);
    } else {
      // Color based on frame kind
      const color = this.getFrameKindColor(frame.frame.frame_kind);
      line.style.color = color;
      line.textContent = frame.text;
    }

    // Add data attribute for search
    line.dataset.lineIndex = String(lineIndex);
    line.dataset.frameKind = frame.frame.frame_kind;

    return line;
  }

  /**
   * Apply span styles for ANSI-colored text.
   */
  private applySpanStyles(
    container: HTMLElement,
    text: string,
    spans: Array<{ start: number; styles: TextStyle[] }>,
  ): void {
    // For now, just display the text with basic styling
    // A full implementation would create span elements for each style range
    container.textContent = text;

    // Apply foreground color from first span if present
    if (spans.length > 0) {
      const firstSpan = spans[0];
      for (const style of firstSpan.styles) {
        if (typeof style === "object" && "type" in style && (style as ColorStyle).type === "foreground" && "color" in style) {
          container.style.color = (style as ColorStyle).color;
          break;
        }
      }
    }
  }

  /**
   * Get color for frame kind.
   */
  private getFrameKindColor(kind: string): string {
    switch (kind) {
      case "stderr":
        return "#f85149"; // Red
      case "log":
        return "#8b949e"; // Gray
      case "prompt":
        return "#58a6ff"; // Blue
      case "status":
        return "#d29922"; // Yellow
      default:
        return "#c9d1d9"; // Default text
    }
  }

  /**
   * Update status display.
   */
  private updateStatus(buffer: ScrollbackBuffer): void {
    const renderer = this.renderer;
    const statusSpan = this.statusSpan;
    if (!renderer || !statusSpan) return;

    const metrics = renderer.getMetrics();
    const fps = metrics.fps > 0 ? `${metrics.fps} fps` : "idle";
    const memory = this.formatMemory(metrics.memoryBytes);
    const association = this.associationInfo;
    const contextParts: string[] = [];
    if (association) {
      if (association.run_id) contextParts.push(`run:${association.run_id}`);
      if (association.command_id) contextParts.push(`cmd:${association.command_id}`);
      if (association.issue_id) contextParts.push(`issue:${association.issue_id}`);
      if (association.sub_issue_id) contextParts.push(`sub:${association.sub_issue_id}`);
      if (association.workspace_id) contextParts.push(`ws:${association.workspace_id}`);
    }
    const context = contextParts.length > 0 ? ` | ${contextParts.join(" ")}` : "";
    statusSpan.textContent = `${buffer.totalFrames} frames | ${fps} | ${memory} | ${buffer.visibleFrames.length} visible${context}`;
  }

  /**
   * Format memory bytes to human-readable string.
   */
  private formatMemory(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  }

  /**
   * Scroll to bottom of terminal output.
   */
  private scrollToBottom(): void {
    const scrollContainer = this.scrollContainer;
    if (!scrollContainer) return;

    requestAnimationFrame(() => {
      if (this.scrollContainer !== scrollContainer) return;
      scrollContainer.scrollTop = scrollContainer.scrollHeight;
    });
  }

  /**
   * Keep renderer scrollback state aligned with manual DOM scrolling.
   *
   * The DOM only renders a small window, so "scrolled to bottom" means latest
   * only when the rendered bottom line is also the newest retained frame.
   */
  private syncScrollStateFromDom(): void {
    const renderer = this.renderer;
    const scrollContainer = this.scrollContainer;
    if (!renderer || !scrollContainer || this.lineElements.length === 0) return;

    const buffer = renderer.getBuffer();
    const lastRenderedIndex = this.getLineIndex(
      this.lineElements[this.lineElements.length - 1],
    );
    const domAtBottom = scrollContainer.scrollTop + scrollContainer.clientHeight
      >= scrollContainer.scrollHeight - 2;
    const atLatest = domAtBottom
      && lastRenderedIndex !== undefined
      && lastRenderedIndex >= buffer.totalFrames - 1;

    if (atLatest) {
      renderer.syncScrollPosition(lastRenderedIndex, true);
      this.updateStatus(renderer.getBuffer());
      return;
    }

    const firstVisibleIndex = this.getFirstVisibleLineIndex();
    if (firstVisibleIndex === undefined) return;

    renderer.syncScrollPosition(firstVisibleIndex, false);
    this.updateStatus(renderer.getBuffer());
  }

  private getFirstVisibleLineIndex(): number | undefined {
    const scrollContainer = this.scrollContainer;
    if (!scrollContainer) return undefined;

    const containerRect = scrollContainer.getBoundingClientRect();
    for (const line of this.lineElements) {
      const lineRect = line.getBoundingClientRect();
      if (lineRect.bottom >= containerRect.top && lineRect.top <= containerRect.bottom) {
        return this.getLineIndex(line);
      }
    }

    return this.getLineIndex(this.lineElements[this.lineElements.length - 1]);
  }

  private getLineIndex(line: HTMLElement | undefined): number | undefined {
    if (!line) return undefined;

    const index = Number(line.dataset.lineIndex);
    return Number.isFinite(index) ? index : undefined;
  }

  /**
   * Perform text search in terminal output.
   * Searches the full scrollback buffer history, not just visible DOM elements.
   */
  private performSearch(): void {
    const renderer = this.renderer;
    const searchInput = this.searchInput;
    if (!renderer || !searchInput) return;

    this.searchTerm = searchInput.value.trim();

    // Clear previous search highlights
    this.clearSearchHighlights();

    // Reset search state for new query
    this.searchResults = [];
    this.currentSearchIndex = 0;
    this.pendingFocusFrameIndex = undefined;

    if (!this.searchTerm) {
      return;
    }

    // Search the full scrollback buffer history via the renderer.
    const buffer = renderer.getBuffer();
    this.searchResults = searchText(buffer, this.searchTerm, false);

    // Highlight first result if it's in visible range
    if (this.searchResults.length > 0) {
      this.highlightCurrentSearchResult();
    }
  }

  /**
   * Clear search highlights.
   */
  private clearSearchHighlights(): void {
    for (const line of this.lineElements) {
      line.style.outline = "";
      line.style.backgroundColor = "";
    }
  }

  /**
   * Highlight current search result.
   * Scrolls the buffer to show the matching frame and highlights it in DOM.
   */
  private highlightCurrentSearchResult(): void {
    if (this.searchResults.length === 0) return;

    // Clear previous highlights
    this.clearSearchHighlights();

    // Highlight current result
    const currentIndex = this.currentSearchIndex % this.searchResults.length;
    const frameIndex = this.searchResults[currentIndex];
    this.pendingFocusFrameIndex = frameIndex;

    // Use renderer's scrollTo to bring the frame into view
    this.renderer?.scrollToFrame(frameIndex);

    // Apply visual highlight to the matching DOM element after render.
    requestAnimationFrame(() => this.highlightLine(frameIndex));
  }

  /**
   * Highlight a specific global frame index if it is visible.
   */
  private highlightLine(frameIndex: number): void {
    const targetElement = this.lineElements.find((line) => line.dataset.lineIndex === String(frameIndex));
    if (!targetElement) return;

    targetElement.style.outline = "2px solid #58a6ff";
    targetElement.style.backgroundColor = "rgba(88, 166, 255, 0.2)";
    targetElement.scrollIntoView({ block: "center" });
  }

  /**
   * Copy terminal output to clipboard.
   * Uses the full scrollback buffer to avoid losing pruned data.
   */
  private copyToClipboard(): void {
    const renderer = this.renderer;
    const copyButton = this.copyButton;
    if (!renderer || !copyButton) return;

    // Get text from full scrollback buffer, not just visible DOM
    const buffer = renderer.getBuffer();
    const text = buffer.allFrames
      .map((frame) => frame.text)
      .join("\n");

    navigator.clipboard.writeText(text).then(() => {
      const originalText = copyButton.textContent;
      copyButton.textContent = "Copied!";
      setTimeout(() => {
        if (this.copyButton === copyButton) {
          copyButton.textContent = originalText;
        }
      }, 1500);
    }).catch(() => {
      // Fallback for browsers that don't support clipboard API
      const textarea = document.createElement("textarea");
      textarea.value = text;
      textarea.style.position = "fixed";
      textarea.style.opacity = "0";
      document.body.appendChild(textarea);
      textarea.select();
      document.execCommand("copy");
      document.body.removeChild(textarea);
    });
  }

  /**
   * Jump to latest output (bottom of scrollback).
   * Updates visible frames to show the most recent content.
   */
  private jumpToLatest(): void {
    this.pendingFocusFrameIndex = undefined;
    this.renderer?.jumpToLatest();
  }

  /**
   * Scroll to a specific frame index and center the visible window on it.
   * Public API for programmatic navigation (e.g., keyboard shortcuts, direct links).
   */
  scrollToFrame(targetIndex: number): void {
    this.pendingFocusFrameIndex = targetIndex;
    this.renderer?.scrollToFrame(targetIndex);
  }

  /**
   * Dispose of the viewer and remove event listeners.
   */
  dispose(): void {
    if (this.searchButton && this._onSearch) {
      this.searchButton.removeEventListener("click", this._onSearch);
    }
    if (this.searchInput && this._onSearchKeypress) {
      this.searchInput.removeEventListener("keypress", this._onSearchKeypress);
    }
    if (this.copyButton && this._onCopy) {
      this.copyButton.removeEventListener("click", this._onCopy);
    }
    if (this.jumpButton && this._onJump) {
      this.jumpButton.removeEventListener("click", this._onJump);
    }
    if (this.scrollContainer && this._onScroll) {
      this.scrollContainer.removeEventListener("scroll", this._onScroll);
    }
  }

  /**
   * Update the association metadata shown in the viewer status bar so the
   * terminal output can be traced back to its run, command, issue, and sub-issue.
   */
  setAssociationInfo(association: TerminalLogAssociation): void {
    this.associationInfo = association;
    const buffer = this.renderer?.getBuffer();
    if (buffer) {
      this.updateStatus(buffer);
    }
  }

  /**
   * Jump to the frame produced by a specific journal event id.
   * Returns true if the event was found in the current scrollback.
   */
  jumpToEvent(eventId: string): boolean {
    const renderer = this.renderer;
    if (!renderer) return false;
    const buffer = renderer.getBuffer();
    const pos = buffer.allFrames.findIndex(
      (decoded) => decoded.frame.source_event_id === eventId,
    );
    if (pos < 0) return false;

    const totalIndex = buffer.totalFrames - buffer.allFrames.length + pos;
    this.pendingFocusFrameIndex = totalIndex;
    renderer.scrollToFrame(totalIndex);
    return true;
  }

  destroy(): void {
    this.dispose();
    this.detachRender?.();
    this.detachRender = undefined;
    this.renderer = null;
    this.container.replaceChildren();
    this.lineElements = [];
    this.searchResults = [];
    this.pendingFocusFrameIndex = undefined;
    this.associationInfo = undefined;
    this.toolbar = undefined;
    this.searchInput = undefined;
    this.searchButton = undefined;
    this.copyButton = undefined;
    this.jumpButton = undefined;
    this.statusSpan = undefined;
    this.scrollContainer = undefined;
    this._onSearch = undefined;
    this._onSearchKeypress = undefined;
    this._onCopy = undefined;
    this._onJump = undefined;
    this._onScroll = undefined;
  }
}

/**
 * Create a terminal viewer instance.
 */
export function createTerminalViewer(
  renderer: TerminalRenderer,
  options: TerminalViewerOptions,
): TerminalViewer {
  return new TerminalViewer(renderer, options);
}
