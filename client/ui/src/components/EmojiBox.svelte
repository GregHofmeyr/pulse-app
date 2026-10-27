<script lang="ts" module>
  /** A text and where its caret goes. */
  export type Snap = { text: string; caret: number }
</script>

<script lang="ts">
  // A plain-text message box that shows custom emoji as images (the composer's and the edit box's).
  // `text` (what's sent or saved) stays the truth: the box is rendered from it when it mounts and
  // whenever it's changed through `setText`, and typing in the box is read back into it. A parent
  // changes the text only through `setText`, never by writing `text` itself.
  import { untrack } from 'svelte'
  import { wireEmoji } from '../lib/animate.svelte'
  import { offsetOf, positionAt, renderable, renderInto, serialize, tokenSpans } from '../lib/composerDom'
  import { mediaUrl } from '../lib/emoji'
  import { app } from '../lib/store.svelte'

  let {
    text = $bindable(''),
    label,
    placeholder = '',
    max = 4000,
    class: cls = '',
    onkeydown,
    onkeyup,
    onclick,
    oninput,
  }: {
    text?: string
    /** The box's accessible name. */
    label: string
    /** Shown while the box is empty. */
    placeholder?: string
    /** The most characters the text may hold: input that doesn't fit is refused. */
    max?: number
    class?: string
    /** Keys after the box's own (IME, undo/redo); call preventDefault to take one. Shift+Enter, if
     *  not taken, adds a line. */
    onkeydown?: (e: KeyboardEvent) => void
    onkeyup?: (e: KeyboardEvent) => void
    onclick?: () => void
    /** After typing changed `text` (`now` is when). */
    oninput?: (now: number) => void
  } = $props()

  let box: HTMLElement
  /** Where the selection was in the text, kept while focus is elsewhere (e.g. in the emoji picker). */
  let sel = { start: 0, end: 0 }
  /** Focus back in the box (from the picker, Tab, a closed dialog): the caret goes where it was (or
   *  where a Shift-pick moved it), unless a click in the box brought focus and placed it. Browsers
   *  put the caret at the start whenever focus arrives without a click, so that can't be the signal. */
  let pressed = false
  function focused() {
    const byClick = pressed
    pressed = false
    if (byClick) return
    queueMicrotask(() => {
      if (document.activeElement === box) placeCaret(sel.start)
    })
  }
  /** The box's own undo history: the browser's can't follow a box we rebuild. */
  let history: Snap[] = []
  let future: Snap[] = []
  let typedAt = 0
  function record(prev: Snap) {
    if (history.at(-1)?.text !== prev.text) history = [...history.slice(-99), prev]
    future = []
  }
  function undo(back: boolean) {
    typedAt = 0 // typing after this is a step of its own
    const from = back ? history : future
    const to = back ? future : history
    const snap = from.pop()
    if (!snap) return
    to.push({ text, caret: sel.start })
    setText(snap, true, false)
  }
  /** Animation wiring of the images now in the box. */
  let unwire: (() => void)[] = []
  const lookup = (id: string) => app.state.customEmoji[id]
  function render(next: string) {
    unwire.forEach((u) => u())
    unwire = renderInto(box, next, lookup).flatMap((img) =>
      img.dataset.anim ? [wireEmoji(img, mediaUrl(img.dataset.anim), img.getAttribute('src') ?? '')] : [],
    )
  }
  /** The selection as text offsets, if it's in the box. */
  export function readSel(): { start: number; end: number } | null {
    const s = getSelection()
    if (!s?.rangeCount || !box?.contains(s.anchorNode)) return null
    const r = s.getRangeAt(0)
    return { start: offsetOf(box, r.startContainer, r.startOffset), end: offsetOf(box, r.endContainer, r.endOffset) }
  }
  /** The selection as last seen in the box (kept while focus is elsewhere). */
  export const selection = () => ({ ...sel })
  function placeCaret(at: number) {
    const p = positionAt(box, at)
    const r = document.createRange()
    r.setStart(p.node, p.offset)
    r.collapse(true)
    const s = getSelection()
    s?.removeAllRanges()
    s?.addRange(r)
  }
  function syncSel() {
    const r = readSel()
    if (r) sel = r
  }
  $effect(() => {
    // untracked: rendering reads app.state (the emoji), which changes on every event
    untrack(() => render(text))
    const onSel = () => {
      // only while typing here: a rebuild while focus is in the picker collapses the selection to 0
      if (document.activeElement !== box) return
      syncSel()
    }
    document.addEventListener('selectionchange', onSel)
    return () => {
      document.removeEventListener('selectionchange', onSel)
      unwire.forEach((u) => u())
    }
  })

  // Emoji the app learns about after the text showed (Ready came later): show them now. Only when
  // there's one to show, so ordinary updates leave the box (and its caret) alone.
  // (the emoji map alone: app.state is replaced on every event, the map only when emoji change)
  const knownEmoji = $derived(app.state.customEmoji)
  $effect(() => {
    const known = knownEmoji
    untrack(() => {
      const shownTokens = new Set([...box.querySelectorAll<HTMLImageElement>('img')].map((i) => i.dataset.token))
      const waiting = tokenSpans(text).some((t) => known[t.id] && renderable(known[t.id]) && !shownTokens.has(text.slice(t.start, t.end)))
      if (!waiting) return
      const at = readSel()?.start
      render(text)
      if (at !== undefined) placeCaret(at)
    })
  })

  /** Replace the text (an emoji or mention picked, a line added, a paste) and put the caret at `r.caret`.
   *  Without `focus` (a Shift-pick in the picker) focus stays where it is and the caret waits in the
   *  kept selection. Without `remember` it's no step for undo. A text over the limit isn't taken:
   *  returns whether it was. */
  export function setText(r: Snap, focus = true, remember = true): boolean {
    if (r.text.length > max) return false // callers check; a change that doesn't fit isn't made
    if (remember) record({ text, caret: sel.start })
    typedAt = 0
    text = r.text
    const caret = Math.min(r.caret, text.length)
    sel = { start: caret, end: caret }
    render(text)
    void Promise.resolve().then(() => {
      if (!focus || !box?.isConnected) return
      box.focus()
      placeCaret(caret)
    })
    return true
  }
  /** Start the undo history afresh (the text was sent). */
  export function forget() {
    history = []
    future = []
  }
  /** Focus the box, the caret at `at` (where it was, if not given). */
  export function focus(at?: number) {
    if (at !== undefined) sel = { start: at, end: at }
    box?.focus()
    // focus() above restores the kept caret (a microtask); this covers a box that already had focus
    if (at !== undefined && document.activeElement === box) placeCaret(at)
  }

  /** Characters free for something replacing `r`. */
  const room = (r: { start: number; end: number }) => max - (text.length - (r.end - r.start))
  /** `insert` as typed (no trailing space) over the selection, cut to what fits (never mid-character). */
  function insertRaw(insert: string) {
    const r = readSel() ?? sel
    let fit = ''
    for (const ch of insert) {
      if (fit.length + ch.length > room(r)) break
      fit += ch
    }
    if (fit) setText({ text: text.slice(0, r.start) + fit + text.slice(r.end), caret: r.start + fit.length })
  }
  /** Only text comes in (a paste or drop of HTML would put markup in the box); a no-break space is a space. */
  const plain = (t: string) => t.replace(/\r\n?/g, '\n').replace(/ /g, ' ')
  function paste(e: ClipboardEvent) {
    e.preventDefault()
    const t = e.clipboardData?.getData('text/plain') ?? ''
    if (t) insertRaw(plain(t))
  }
  /** Text being dragged from inside the box: a drop back in moves it. */
  let dragFrom: { start: number; end: number } | null = null
  /** The text offset under the pointer, where the browser can say. */
  function offsetAtPoint(x: number, y: number): number | null {
    const d = document as Document & {
      caretPositionFromPoint?: (x: number, y: number) => { offsetNode: Node; offset: number } | null
      caretRangeFromPoint?: (x: number, y: number) => Range | null
    }
    const p = d.caretPositionFromPoint?.(x, y)
    if (p && box.contains(p.offsetNode)) return offsetOf(box, p.offsetNode, p.offset)
    const r = d.caretRangeFromPoint?.(x, y)
    if (r && box.contains(r.startContainer)) return offsetOf(box, r.startContainer, r.startOffset)
    return null
  }
  /** Marks a drag that started in this box (a dragFrom left by a lost dragend can't pass for one). */
  const OWN_DRAG = 'application/x-pulse-composer'
  function dragStart(e: DragEvent) {
    const r = readSel()
    dragFrom = r && r.start !== r.end ? r : null
    if (!dragFrom) return
    // the text itself, tokens included (the browser would give the images' alt text)
    e.dataTransfer?.setData('text/plain', text.slice(dragFrom.start, dragFrom.end))
    e.dataTransfer?.setData(OWN_DRAG, '1')
  }
  function drop(e: DragEvent) {
    e.preventDefault()
    const from = e.dataTransfer?.getData(OWN_DRAG) ? dragFrom : null
    dragFrom = null
    // a move takes the box's own text; anything else, what was dropped
    const t = from ? text.slice(from.start, from.end) : plain(e.dataTransfer?.getData('text/plain') ?? '')
    if (!t) return
    let at = offsetAtPoint(e.clientX, e.clientY) ?? (readSel() ?? sel).start
    let base = text
    if (from) {
      // a move: out of where it was (unless dropped back inside it), then in where it lands
      if (at >= from.start && at <= from.end) return
      base = text.slice(0, from.start) + text.slice(from.end)
      if (at > from.end) at -= from.end - from.start
    } else if (t.length > max - text.length) {
      return
    }
    setText({ text: base.slice(0, at) + t + base.slice(at), caret: at + t.length })
  }
  /** The clipboard gets the text itself, tokens included: the browser would give an image's alt. */
  function copy(e: ClipboardEvent, cut = false) {
    const r = readSel()
    if (!r || r.start === r.end) return
    e.preventDefault()
    e.clipboardData?.setData('text/plain', text.slice(r.start, r.end))
    if (cut) setText({ text: text.slice(0, r.start) + text.slice(r.end), caret: r.start })
  }
  function beforeInput(e: InputEvent) {
    // plain text only: bold/italic would show but never be sent
    if (e.inputType.startsWith('format')) return e.preventDefault()
    if (e.inputType === 'historyUndo' || e.inputType === 'historyRedo') {
      e.preventDefault()
      return undo(e.inputType === 'historyUndo')
    }
    // the limit refuses what's coming in, never what's there (an IME commit is checked after, in input())
    if (!e.isComposing && e.data && e.data.length > room(readSel() ?? sel)) e.preventDefault()
  }

  function key(e: KeyboardEvent) {
    // keys for an IME's candidate list (WebKit can say 229 without isComposing) are the IME's
    if (e.isComposing || e.keyCode === 229) return
    // undo/redo are ours (the browser's history can't follow a box we rebuild, and once we've
    // taken its undo it has no redo to offer, so its beforeinput never comes)
    const mod = e.ctrlKey || e.metaKey
    const k = e.key.toLowerCase()
    if (mod && !e.altKey && (k === 'z' || k === 'y')) {
      e.preventDefault()
      return undo(k === 'z' && !e.shiftKey)
    }
    onkeydown?.(e)
    if (e.defaultPrevented) return
    if (e.key === 'Enter' && e.shiftKey) {
      // a newline in the text itself (the browser would wrap lines in elements)
      e.preventDefault()
      insertRaw('\n')
    }
  }

  function input(e: Event) {
    const composing = (e as InputEvent).isComposing
    const next = serialize(box)
    if (next.length > max) {
      // got past the limit (an IME commit): back to the text as it was, caret where it was
      render(text)
      placeCaret(sel.start)
      return
    }
    // typing is remembered for undo in bursts, not per key
    const now = Date.now()
    if (now - typedAt > 1000) record({ text, caret: sel.start })
    typedAt = now
    text = next
    if (!composing) {
      const at = readSel()?.start ?? next.length
      // the browser leaves a lone <br> in a box it emptied (clear it, so the placeholder shows), and
      // can drop the line break a text ending in a newline needs (or its last line disappears)
      if ((!next && box.childNodes.length) || (next.endsWith('\n') && !(box.lastChild instanceof HTMLBRElement))) {
        render(next)
        placeCaret(at)
      }
    }
    syncSel()
    oninput?.(now)
  }
</script>

<div bind:this={box} class="box {cls}" class:empty={!text} contenteditable="plaintext-only" role="textbox" tabindex="0" dir="auto"
  aria-multiline="true" aria-label={label} aria-placeholder={placeholder || undefined} data-placeholder={placeholder}
  onkeydown={key} onbeforeinput={beforeInput} oninput={input} onpaste={paste} oncopy={copy} oncut={(e) => copy(e, true)}
  onfocus={focused} onpointerdown={() => (pressed = document.activeElement !== box)}
  onpointerup={(e) => { if (e.pointerType === 'mouse') pressed = false }} onpointercancel={() => (pressed = false)}
  ondragstart={dragStart} ondragend={() => (dragFrom = null)} ondrop={drop}
  onclick={() => { pressed = false; syncSel(); onclick?.() }} {onkeyup}></div>

<style>
  /* grows with what's in it (images included) up to the max-height its user gives, then scrolls */
  .box { width: 100%; box-sizing: border-box; overflow-y: auto; color: var(--text); outline: none; white-space: pre-wrap; overflow-wrap: anywhere; line-height: 20px; cursor: text; }
  /* the alt text after the slash keeps the placeholder out of the box's accessible content; the
     line before it is for engines without that syntax (they drop the second line) */
  .box.empty::before { content: attr(data-placeholder); content: attr(data-placeholder) / ''; color: var(--text-3); pointer-events: none; }
  .box :global(img.emoji) { width: 22px; height: 22px; vertical-align: -5px; margin: 0 1px; object-fit: contain; }
</style>
