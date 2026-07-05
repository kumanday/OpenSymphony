/**
 * Minimal dependency-free DOM morphing.
 *
 * `morphChildren` reconciles a live element's children against a new HTML
 * string, mutating only what changed instead of rebuilding the subtree via
 * `innerHTML`. Unchanged nodes keep their identity, so focus, text
 * selection, scroll offsets, canvas bitmaps, and attached event listeners
 * survive a re-render.
 *
 * Reconciliation rules:
 * - Nodes match by position when they share node type and tag name.
 * - Elements carrying an identity attribute (`id`, `data-key`,
 *   `data-node-id`, or `data-path`) only match a node with the same key and
 *   are searched for out of position, so keyed list items move instead of
 *   being rewritten in place.
 * - Form fields that currently hold focus keep their user-entered value,
 *   checked state, and selection; everything else syncs to the new render.
 */

const KEY_ATTRIBUTES = ["id", "data-key", "data-node-id", "data-path", "data-approval-id"] as const;

export function morphChildren(target: Element, html: string): void {
  const template = target.ownerDocument.createElement("template");
  template.innerHTML = html;
  morphChildNodes(target, template.content);
}

function morphChildNodes(target: ParentNode & Node, source: ParentNode): void {
  const sourceChildren = Array.from(source.childNodes);

  for (let index = 0; index < sourceChildren.length; index += 1) {
    const sourceChild = sourceChildren[index];
    const existing = target.childNodes[index] ?? null;

    const keyed = findKeyedMatch(target, index, sourceChild);
    if (keyed && keyed !== existing) {
      target.insertBefore(keyed, existing);
    }

    const current = target.childNodes[index] ?? null;
    if (!current) {
      target.appendChild(sourceChild);
      continue;
    }
    if (isCompatible(current, sourceChild)) {
      morphNode(current, sourceChild);
    } else {
      target.replaceChild(sourceChild, current);
    }
  }

  while (target.childNodes.length > sourceChildren.length) {
    target.removeChild(target.lastChild!);
  }
}

function findKeyedMatch(target: ParentNode, fromIndex: number, sourceChild: Node): Element | null {
  if (sourceChild.nodeType !== 1) {
    return null;
  }
  const key = nodeKey(sourceChild as Element);
  if (key === null) {
    return null;
  }
  for (let index = fromIndex; index < target.childNodes.length; index += 1) {
    const candidate = target.childNodes[index];
    if (
      candidate.nodeType === 1
      && candidate.nodeName === sourceChild.nodeName
      && nodeKey(candidate as Element) === key
    ) {
      return candidate as Element;
    }
  }
  return null;
}

function nodeKey(element: Element): string | null {
  for (const attribute of KEY_ATTRIBUTES) {
    const value = element.getAttribute(attribute);
    if (value !== null && value !== "") {
      return `${attribute}=${value}`;
    }
  }
  // Canvas identity matters (a redrawn canvas loses its bitmap), and canvases
  // in this app are addressed by test id rather than a list key.
  if (element.nodeName === "CANVAS") {
    const testId = element.getAttribute("data-testid");
    if (testId) {
      return `data-testid=${testId}`;
    }
  }
  return null;
}

function isCompatible(current: Node, next: Node): boolean {
  if (current.nodeType !== next.nodeType) {
    return false;
  }
  if (current.nodeType !== 1) {
    return true;
  }
  if (current.nodeName !== next.nodeName) {
    return false;
  }
  return nodeKey(current as Element) === nodeKey(next as Element);
}

function morphNode(current: Node, next: Node): void {
  if (current.nodeType === 3 || current.nodeType === 8) {
    if (current.nodeValue !== next.nodeValue) {
      current.nodeValue = next.nodeValue;
    }
    return;
  }
  if (current.nodeType !== 1) {
    return;
  }
  const currentElement = current as Element;
  const nextElement = next as Element;
  const focused = currentElement.ownerDocument.activeElement === currentElement;

  if (currentElement.nodeName === "CANVAS") {
    // Canvases are imperative surfaces: their bitmap, dimensions, and inline
    // style are managed by the code drawing on them. Only add or update
    // attributes the new render declares; never remove the rest (removing
    // width/height would clear the bitmap).
    for (const attribute of Array.from(nextElement.attributes)) {
      if (currentElement.getAttribute(attribute.name) !== attribute.value) {
        currentElement.setAttribute(attribute.name, attribute.value);
      }
    }
    return;
  }

  if (currentElement.hasAttribute("data-morph-ignore-children")) {
    // Imperative islands (e.g. the knowledge-graph overlay layer) own their
    // children outside the render cycle; only their attributes sync.
    syncAttributes(currentElement, nextElement, focused);
    return;
  }

  // Capture the target form state before morphing children: reconciliation
  // can move option nodes out of `next`, which would change its `value`.
  const formState = captureFormState(nextElement);
  syncAttributes(currentElement, nextElement, focused);
  if (currentElement.nodeName === "TEXTAREA") {
    // A textarea's child text is its default value; sync it as plain text and
    // let applyFormState decide whether the live value follows.
    if (currentElement.textContent !== nextElement.textContent) {
      currentElement.textContent = nextElement.textContent;
    }
  } else {
    morphChildNodes(currentElement, nextElement);
  }
  applyFormState(currentElement, formState, focused);
}

function syncAttributes(current: Element, next: Element, focused: boolean): void {
  for (const attribute of Array.from(current.attributes)) {
    if (focused && (attribute.name === "value" || attribute.name === "checked")) {
      continue;
    }
    if (!next.hasAttribute(attribute.name)) {
      current.removeAttribute(attribute.name);
    }
  }
  for (const attribute of Array.from(next.attributes)) {
    if (focused && (attribute.name === "value" || attribute.name === "checked")) {
      continue;
    }
    if (current.getAttribute(attribute.name) !== attribute.value) {
      current.setAttribute(attribute.name, attribute.value);
    }
  }
}

interface FormState {
  value?: string;
  checked?: boolean;
  selected?: boolean;
}

/**
 * Live form state (`value`, `checked`, `selected`) diverges from the
 * rendered attributes as soon as the user interacts with a field, so it must
 * be written back explicitly — unless the field holds focus, in which case
 * the user's in-progress input wins over the render.
 */
function captureFormState(next: Element): FormState {
  if (next instanceof HTMLInputElement) {
    return { value: next.value, checked: next.checked };
  }
  if (next instanceof HTMLTextAreaElement || next instanceof HTMLSelectElement) {
    return { value: next.value };
  }
  if (next instanceof HTMLOptionElement) {
    return { selected: next.selected };
  }
  return {};
}

function applyFormState(current: Element, state: FormState, focused: boolean): void {
  if (focused) {
    return;
  }
  if (current instanceof HTMLInputElement) {
    if (state.value !== undefined && current.value !== state.value) {
      current.value = state.value;
    }
    if (state.checked !== undefined && current.checked !== state.checked) {
      current.checked = state.checked;
    }
    return;
  }
  if (current instanceof HTMLTextAreaElement || current instanceof HTMLSelectElement) {
    if (state.value !== undefined && current.value !== state.value) {
      current.value = state.value;
    }
    return;
  }
  if (current instanceof HTMLOptionElement && state.selected !== undefined) {
    if (current.selected !== state.selected) {
      current.selected = state.selected;
    }
  }
}
