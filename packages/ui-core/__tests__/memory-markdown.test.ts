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
});
