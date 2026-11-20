// Message edits still open with changes (#460): the edit box's text lives only in that box, so an update never installs
// by itself over one. Each open MessageItem marks its own (by a key of its own) and unmarks it when it closes or goes.
const unsaved = new Set<symbol>()

/** `key`'s edit box is open with changes (`true`), or closed / unchanged / gone (`false`). */
export function markEdit(key: symbol, changed: boolean): void {
  if (changed) unsaved.add(key)
  else unsaved.delete(key)
}

/** Some message edit is open with changes nobody saved. */
export function hasUnsavedEdits(): boolean {
  return unsaved.size > 0
}
