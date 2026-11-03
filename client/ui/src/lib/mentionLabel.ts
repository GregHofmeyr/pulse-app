// What a mention pill or chip says (#137): `@` in its own `.at` span, then the name, so CSS can put a thin gap between
// them (crowded otherwise, worse with name fonts). The text stays `@name`: copying, `textContent` and screen readers
// see no change.

/** Fills `el` with `@` + `name`. */
export function setMentionLabel(el: HTMLElement, name: string): void {
  const at = el.ownerDocument.createElement('span')
  at.className = 'at'
  at.textContent = '@'
  el.replaceChildren(at, name)
}
