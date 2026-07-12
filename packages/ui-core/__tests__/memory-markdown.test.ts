import { renderMemoryMarkdown } from "../src/memory-markdown.js";

describe("renderMemoryMarkdown", () => {
  it("renders headings, paragraphs, and lists at inspector scale", () => {
    const html = renderMemoryMarkdown([
      "## Summary",
      "",
      "First line",
      "continued line.",
      "",
      "- item one",
      "- item two",
      "",
      "# Top",
      "### Deep",
    ].join("\n"));
    expect(html).toContain("<h5>Summary</h5>");
    expect(html).toContain("<p>First line continued line.</p>");
    expect(html).toContain("<ul><li>item one</li><li>item two</li></ul>");
    expect(html).toContain("<h4>Top</h4>");
    expect(html).toContain("<h6>Deep</h6>");
  });

  it("escapes HTML before reintroducing allowlisted constructs", () => {
    const html = renderMemoryMarkdown("<script>alert(1)</script> **bold** `code`");
    expect(html).not.toContain("<script>");
    expect(html).toContain("&lt;script&gt;");
    expect(html).toContain("<strong>bold</strong>");
    expect(html).toContain("<code>code</code>");
  });

  it("renders wiki links as graph navigation buttons", () => {
    const html = renderMemoryMarkdown("Links to [[issues/COE-399]] inline.");
    expect(html).toContain(`data-kg-link-target="issues/COE-399"`);
    expect(html).toContain(">COE-399</button>");
  });

  it("allows only http(s) markdown links and neutralizes everything else", () => {
    const safe = renderMemoryMarkdown("[docs](https://example.com/a?b=1)");
    expect(safe).toContain(`<a href="https://example.com/a?b=1" target="_blank" rel="noreferrer">docs</a>`);

    // Non-http(s) schemes stay inert plain text: no anchor, no href.
    const hostile = renderMemoryMarkdown("[x](javascript:alert(1))");
    expect(hostile).not.toContain("<a ");
    expect(hostile).not.toContain("href");

    const attr = renderMemoryMarkdown(`[["><img src=x onerror=alert(1)>]]`);
    expect(attr).not.toContain("<img");
    expect(attr).toContain("&gt;");
  });

  it("renders valid code deep links as navigation buttons", () => {
    const html = renderMemoryMarkdown("[Open symbol](opensymphony://code/repo/symbols/foo)");
    expect(html).toContain('data-code-deeplink="opensymphony://code/repo/symbols/foo"');
    expect(html).toContain('class="os-kg-capsule-link"');
    expect(html).not.toContain('href="opensymphony://');
  });

  it("preserves query parameters in code deep-link buttons", () => {
    const html = renderMemoryMarkdown("[Open symbol](opensymphony://code/repo/atlas?depth=2&seed=x)");
    expect(html).toContain('data-code-deeplink="opensymphony://code/repo/atlas?depth=2&amp;seed=x"');
    expect(html).not.toContain("&amp;amp;");
  });

  it("never reprocesses generated HTML with later inline passes", () => {
    // A wiki target containing markdown-link syntax must land verbatim in
    // the attribute, not be rewritten into a nested anchor.
    const html = renderMemoryMarkdown("[[foo [docs](https://example.com)]]");
    expect(html).toContain(`data-kg-link-target="foo [docs](https://example.com)"`);
    expect(html).not.toContain("<a ");

    // Bold/code markers inside link labels and hrefs also stay verbatim.
    const bold = renderMemoryMarkdown("[**label**](https://example.com/a**b**c)");
    expect(bold).toContain(`href="https://example.com/a**b**c"`);
    expect(bold).not.toContain("<strong>");

    // Stray NUL bytes in the source cannot alias placeholder tokens.
    const nul = renderMemoryMarkdown("a\u00001\u0000b **x**");
    expect(nul).toContain("a1b");
    expect(nul).toContain("<strong>x</strong>");
  });

  it("keeps code span content literal against other inline syntax", () => {
    // Code spans run first: markdown-looking content inside backticks must
    // render as literal code, never as placeholders or nested markup.
    const bold = renderMemoryMarkdown("run `**flag**` now");
    expect(bold).toContain("<code>**flag**</code>");
    expect(bold).not.toContain("\u0000");
    expect(bold).not.toContain("<strong>");

    const link = renderMemoryMarkdown("see `[docs](https://example.com)` here");
    expect(link).toContain("<code>[docs](https://example.com)</code>");
    expect(link).not.toContain("<a ");

    // Placeholders nested inside later-stashed fragments still resolve.
    const nested = renderMemoryMarkdown("[[target `code` name]] and `x`");
    expect(nested).not.toContain("\u0000");
    expect(nested).toContain("<code>x</code>");
  });
});
