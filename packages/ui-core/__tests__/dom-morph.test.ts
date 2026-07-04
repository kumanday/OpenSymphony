/**
 * @jest-environment jsdom
 *
 * DOM morphing unit tests: re-renders must mutate only what changed so node
 * identity (focus, listeners, canvas bitmaps) survives.
 */

import { morphChildren } from "../src/dom-morph.js";

function mount(html: string): HTMLElement {
  const root = document.createElement("div");
  document.body.appendChild(root);
  root.innerHTML = html;
  return root;
}

afterEach(() => {
  document.body.replaceChildren();
});

describe("morphChildren", () => {
  it("updates text and attributes in place without replacing the element", () => {
    const root = mount(`<p class="a" data-x="1">before</p>`);
    const paragraph = root.querySelector("p")!;

    morphChildren(root, `<p class="b">after</p>`);

    expect(root.querySelector("p")).toBe(paragraph);
    expect(paragraph.textContent).toBe("after");
    expect(paragraph.className).toBe("b");
    expect(paragraph.hasAttribute("data-x")).toBe(false);
  });

  it("keeps identity of keyed nodes when the list reorders", () => {
    const root = mount(`
      <ul>
        <li data-key="a">A</li>
        <li data-key="b">B</li>
        <li data-key="c">C</li>
      </ul>
    `);
    const itemC = root.querySelector("[data-key='c']")!;

    morphChildren(root, `
      <ul>
        <li data-key="c">C2</li>
        <li data-key="a">A</li>
      </ul>
    `);

    const items = Array.from(root.querySelectorAll("li"));
    expect(items.map((item) => item.dataset.key)).toEqual(["c", "a"]);
    expect(root.querySelector("[data-key='c']")).toBe(itemC);
    expect(itemC.textContent).toBe("C2");
  });

  it("preserves attached event listeners on surviving nodes", () => {
    const root = mount(`<button data-key="go">Go</button>`);
    const button = root.querySelector("button")!;
    let clicks = 0;
    button.addEventListener("click", () => {
      clicks += 1;
    });

    morphChildren(root, `<button data-key="go">Go!</button>`);
    (root.querySelector("button") as HTMLButtonElement).click();

    expect(clicks).toBe(1);
  });

  it("keeps the focused input's value and focus across a morph", () => {
    const root = mount(`<input data-key="search" value="old">`);
    const input = root.querySelector("input")!;
    input.focus();
    input.value = "user typed this";

    morphChildren(root, `<input data-key="search" value="rendered">`);

    expect(document.activeElement).toBe(input);
    expect(input.value).toBe("user typed this");
  });

  it("syncs value on unfocused inputs to the new render", () => {
    const root = mount(`<input data-key="search" value="old">`);
    const input = root.querySelector("input")!;
    input.value = "stale user edit";

    morphChildren(root, `<input data-key="search" value="rendered">`);

    expect(input.value).toBe("rendered");
  });

  it("syncs textarea content and select values", () => {
    const root = mount(`
      <textarea>old text</textarea>
      <select><option value="a" selected>A</option><option value="b">B</option></select>
    `);
    const textarea = root.querySelector("textarea")!;
    const select = root.querySelector("select")!;

    morphChildren(root, `
      <textarea>new text</textarea>
      <select><option value="a">A</option><option value="b" selected>B</option></select>
    `);

    expect(textarea.value).toBe("new text");
    expect(textarea.textContent).toBe("new text");
    expect(select.value).toBe("b");
  });

  it("preserves canvas identity and imperatively-set attributes", () => {
    const root = mount(`<div><canvas data-testid="stage"></canvas></div>`);
    const canvas = root.querySelector("canvas")!;
    canvas.setAttribute("width", "640");
    canvas.setAttribute("height", "480");
    canvas.setAttribute("style", "width: 320px;");

    morphChildren(root, `<div><canvas data-testid="stage" class="lit"></canvas></div>`);

    expect(root.querySelector("canvas")).toBe(canvas);
    expect(canvas.getAttribute("width")).toBe("640");
    expect(canvas.getAttribute("height")).toBe("480");
    expect(canvas.getAttribute("style")).toBe("width: 320px;");
    expect(canvas.className).toBe("lit");
  });

  it("adds and removes nodes to match the new markup", () => {
    const root = mount(`<span>one</span><span>two</span><span>three</span>`);

    morphChildren(root, `<span>one</span><em>replacement</em>`);

    expect(root.children).toHaveLength(2);
    expect(root.children[0].textContent).toBe("one");
    expect(root.children[1].nodeName).toBe("EM");
  });

  it("replaces nodes whose keys differ instead of reusing them", () => {
    const root = mount(`<div data-node-id="alpha">alpha</div>`);
    const alpha = root.querySelector("div")!;

    morphChildren(root, `<div data-node-id="beta">beta</div>`);

    const beta = root.querySelector("div")!;
    expect(beta).not.toBe(alpha);
    expect(beta.dataset.nodeId).toBe("beta");
  });
});
