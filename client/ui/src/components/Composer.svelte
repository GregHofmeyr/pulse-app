<script lang="ts" module>
  // Unsent text per channel, kept while the app runs (switching channels doesn't lose it).
  const drafts = new Map<string, string>()
</script>

<script lang="ts">
  import { untrack } from 'svelte'
  import Icon from './Icon.svelte'
  import EmojiPopover from './EmojiPopover.svelte'
  import { customToken, savedTone, search, twemojiUrl, withSkin, type Std } from '../lib/emoji'
  import { applyEmoji, emojiQuery, insertAt } from '../lib/emojiComplete'
  import { animatedEmoji, stillSrc, wireEmoji } from '../lib/animate.svelte'
  import { offsetOf, positionAt, renderInto, serialize, tokenSpans } from '../lib/composerDom'
  import { mediaUrl } from '../lib/emoji'
  import { typingThrottle } from '../lib/markdown'
  import { applyMention, mentionQuery, suggest, type Mentionable } from '../lib/mentions'
  import type { CustomEmoji } from '../lib/protocol/CustomEmoji'
  import { memberServers } from '../lib/reactions'
  import { app } from '../lib/store.svelte'

  let {
    placeholder,
    draftKey,
    replyingTo,
    onCancelReply,
    onSend,
    onTyping,
    mentionables = [],
  }: {
    placeholder: string
    draftKey: string
    replyingTo: string | null
    onCancelReply: () => void
    onSend: (text: string) => void
    onTyping: () => void
    /** People who can be @mentioned here (autocomplete). */
    mentionables?: Mentionable[]
  } = $props()

  // The composer remounts per channel ({#key} in Shell), so reading the key once is intended.
  let text = $state(untrack(() => drafts.get(draftKey) ?? ''))
  $effect(() => {
    if (text) drafts.set(draftKey, text)
    else drafts.delete(draftKey)
  })
  let lastTyping: number | null = null
  // The message box is editable HTML so custom emoji can show as images; `text` (what's sent and
  // what drafts keep) stays the truth, and the box is rebuilt from it whenever we change it.
  let box: HTMLElement
  const MAX = 4000
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
  type Snap = { text: string; caret: number }
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
    void setText(snap, true, false)
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
  function readSel(): { start: number; end: number } | null {
    const s = getSelection()
    if (!s?.rangeCount || !box?.contains(s.anchorNode)) return null
    const r = s.getRangeAt(0)
    return { start: offsetOf(box, r.startContainer, r.startOffset), end: offsetOf(box, r.endContainer, r.endOffset) }
  }
  function placeCaret(at: number) {
    const p = positionAt(box, at)
    const r = document.createRange()
    r.setStart(p.node, p.offset)
    r.collapse(true)
    const s = getSelection()
    s?.removeAllRanges()
    s?.addRange(r)
  }
  $effect(() => {
    // untracked: rendering reads app.state (the emoji), which changes on every event
    untrack(() => render(text))
    const onSel = () => {
      // only while typing here: a rebuild while focus is in the picker collapses the selection to 0
      if (document.activeElement !== box) return
      const r = readSel()
      if (r) sel = r
    }
    document.addEventListener('selectionchange', onSel)
    return () => {
      document.removeEventListener('selectionchange', onSel)
      unwire.forEach((u) => u())
    }
  })

  // A draft's emoji the app learns about after it showed (Ready came later): show them now. Only
  // when there's one to show, so ordinary updates leave the box (and its caret) alone.
  $effect(() => {
    const known = app.state.customEmoji
    untrack(() => {
      const waiting = tokenSpans(text).some((t) => known[t.id]?.hash && ![...box.querySelectorAll<HTMLImageElement>('img')].some((i) => i.dataset.token === text.slice(t.start, t.end)))
      if (!waiting) return
      const at = readSel()?.start
      render(text)
      if (at !== undefined) placeCaret(at)
    })
  })

  // @-mention popup: open while the caret sits right after an @token that matches someone.
  let caret = $state(0)
  let pick = $state(0)
  const query = $derived(mentionQuery(text.slice(0, caret)))
  const options = $derived(query === null ? [] : suggest(mentionables, query))
  let dismissed = $state<string | null>(null) // Esc closes the popup until the token changes
  const open = $derived(options.length > 0 && dismissed !== query)

  // `:name` popup: custom emoji you can use (your servers) and standard ones. Never open with @mentions:
  // the two queries can't both end at the caret.
  const emojiQ = $derived(emojiQuery(text.slice(0, caret)))
  const usable = $derived.by(() => {
    const mine = memberServers(app.state)
    return Object.values(app.state.customEmoji).filter((c) => mine.has(c.server_id))
  })
  const emojiOptions = $derived(emojiQ === null ? [] : search(emojiQ, usable).slice(0, 8))
  // Re-read as the query changes: a tone picked in the picker meanwhile applies from the next keystroke.
  const tone = $derived(emojiOptions.length ? savedTone() : 0)
  const emojiOpen = $derived(emojiOptions.length > 0 && dismissed !== ':' + emojiQ)
  const shown = $derived(open ? options.length : emojiOpen ? emojiOptions.length : 0)
  const serverName = (id: string) => app.state.servers.find((s) => s.id === id)?.name ?? ''
  const emojiText = (e: Std | CustomEmoji) => ('u' in e ? withSkin(e, tone) : customToken(e))

  let picking = $state(false)
  let emojiBtn: HTMLButtonElement | undefined = $state()

  function syncCaret() {
    const r = readSel()
    if (r) sel = r
    caret = r?.start ?? text.length
    pick = 0
  }

  /** Replace the text (an emoji or mention picked, a line added, a paste) and put the caret at `r.caret`.
   *  Without `focus` (a Shift-pick in the picker) focus stays where it is and the caret waits in `sel`. */
  async function setText(r: Snap, focus = true, remember = true) {
    if (r.text.length > MAX) return // callers check; a change that doesn't fit isn't made
    if (remember) record({ text, caret: sel.start })
    typedAt = 0
    text = r.text
    caret = Math.min(r.caret, text.length)
    sel = { start: caret, end: caret }
    render(text)
    await Promise.resolve()
    if (!focus || !box?.isConnected) return
    box.focus()
    placeCaret(caret)
  }

  const choose = (name: string) => setText(applyMention(text, caret, name))
  const chooseEmoji = (e: Std | CustomEmoji) => setText(applyEmoji(text, caret, emojiText(e)))

  // `sel` keeps the box's selection while the picker has focus: insert there.
  // A `keep` pick (Shift) leaves the picker open and the box unfocused, with the caret advanced.
  function pickEmoji(e: Std | CustomEmoji, { keep }: { keep: boolean }) {
    if (!keep) picking = false
    void setText(insertAt(text, sel.start, sel.end, emojiText(e)), !keep)
  }

  /** Characters free for something replacing `r`. */
  const room = (r: { start: number; end: number }) => MAX - (text.length - (r.end - r.start))
  /** `insert` as typed (no trailing space) over the selection, cut to what fits (never mid-character). */
  function insertRaw(insert: string) {
    const r = readSel() ?? sel
    let fit = ''
    for (const ch of insert) {
      if (fit.length + ch.length > room(r)) break
      fit += ch
    }
    if (fit) void setText({ text: text.slice(0, r.start) + fit + text.slice(r.end), caret: r.start + fit.length })
  }
  /** Only text comes in (a paste or drop of HTML would put markup in the box); a no-break space is a space. */
  const plain = (t: string) => t.replace(/\r\n?/g, '\n').replace(/\u00a0/g, ' ')
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
  function drop(e: DragEvent) {
    e.preventDefault()
    const from = dragFrom
    dragFrom = null
    const t = plain(e.dataTransfer?.getData('text/plain') ?? '')
    if (!t) return
    let at = offsetAtPoint(e.clientX, e.clientY) ?? (readSel() ?? sel).start
    let base = text
    if (from) {
      // a move: out of where it was (unless dropped back inside it), then in where it lands
      if (at >= from.start && at <= from.end) return
      base = text.slice(0, from.start) + text.slice(from.end)
      if (at > from.end) at -= from.end - from.start
    } else if (t.length > MAX - text.length) {
      return
    }
    void setText({ text: base.slice(0, at) + t + base.slice(at), caret: at + t.length })
  }
  /** The clipboard gets the text itself, tokens included: the browser would give an image's alt. */
  function copy(e: ClipboardEvent, cut = false) {
    const r = readSel()
    if (!r || r.start === r.end) return
    e.preventDefault()
    e.clipboardData?.setData('text/plain', text.slice(r.start, r.end))
    if (cut) void setText({ text: text.slice(0, r.start) + text.slice(r.end), caret: r.start })
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
    if (shown) {
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault()
        pick = (pick + (e.key === 'ArrowDown' ? 1 : shown - 1)) % shown
        return
      }
      if (e.key === 'Enter' || e.key === 'Tab') {
        e.preventDefault()
        // a pick that doesn't fit isn't made (setText refuses it)
        void (open ? choose(options[pick].name) : chooseEmoji(emojiOptions[pick]))
        return
      }
      if (e.key === 'Escape') {
        e.preventDefault()
        dismissed = open ? query : ':' + emojiQ
        return
      }
    }
    if (e.key === 'Enter' && e.shiftKey) {
      // a newline in the text itself (the browser would wrap lines in elements)
      e.preventDefault()
      insertRaw('\n')
      return
    }
    if (e.key === 'Enter') {
      e.preventDefault()
      const t = text.trim()
      if (!t) return
      onSend(t)
      void setText({ text: '', caret: 0 }, true, false)
      history = []
      future = []
      lastTyping = null
      return
    }
    if (e.key === 'Escape' && replyingTo) onCancelReply()
  }

  function input(e: Event) {
    const composing = (e as InputEvent).isComposing
    const next = serialize(box)
    if (next.length > MAX) {
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
    syncCaret()
    dismissed = null
    if (text.trim() && typingThrottle(lastTyping, now)) {
      lastTyping = now
      onTyping()
    }
  }

  export function focus() {
    box?.focus()
  }
</script>

<div class="wrap">
  {#if open}
    <div class="mentions" role="listbox" aria-label="Mention someone">
      {#each options as o, i (o.id)}
        <button role="option" aria-selected={i === pick} class:on={i === pick}
          onmousedown={(e) => { e.preventDefault(); void choose(o.name) }}>@{o.name}</button>
      {/each}
    </div>
  {:else if emojiOpen}
    <div class="mentions" role="listbox" aria-label="Insert emoji">
      {#each emojiOptions as o, i ('u' in o ? o.u : o.id)}
        <button role="option" aria-selected={i === pick} class:on={i === pick}
          onmousedown={(e) => { e.preventDefault(); void chooseEmoji(o) }}>
          {#if 'u' in o}
            <img src={twemojiUrl(withSkin(o, tone))} alt="" draggable="false" /><span>:{o.n}:</span>
          {:else}
            <img src={stillSrc(o)} alt="" draggable="false" {@attach animatedEmoji(o)} /><span>:{o.name}:</span>
            <small>{serverName(o.server_id)}</small>
          {/if}
        </button>
      {/each}
    </div>
  {/if}
  {#if replyingTo}
    <div class="replying">Replying to <strong>{replyingTo}</strong>
      <button aria-label="Cancel reply" onclick={onCancelReply}><Icon name="x" size={13} /></button></div>
  {/if}
  <div bind:this={box} class="input" class:empty={!text} contenteditable="plaintext-only" role="textbox" tabindex="0" dir="auto"
    aria-multiline="true" aria-label={placeholder} aria-placeholder={placeholder} data-placeholder={placeholder}
    onkeydown={key} onbeforeinput={beforeInput} oninput={input} onpaste={paste} oncopy={copy} oncut={(e) => copy(e, true)}
    onfocus={focused} onpointerdown={() => (pressed = document.activeElement !== box)}
    onpointerup={() => (pressed = false)} onpointercancel={() => (pressed = false)}
    ondragstart={() => (dragFrom = readSel())} ondragend={() => (dragFrom = null)} ondrop={drop} onclick={syncCaret}
    onkeyup={(e) => { if (e.key.startsWith('Arrow') && !shown) syncCaret() }}></div>
  <button bind:this={emojiBtn} class="emoji-btn" type="button" aria-label="Emoji" data-menu-toggle onclick={() => (picking = !picking)}><Icon name="smile" size={20} /></button>
  {#if picking && emojiBtn}
    <EmojiPopover anchor={emojiBtn} onPick={pickEmoji} onClose={() => (picking = false)} />
  {/if}
</div>

<style>
  .wrap { position: relative; margin: 4px 16px 16px; background: var(--bg-3); border-radius: 12px; }
  .mentions { position: absolute; left: 0; bottom: calc(100% + 6px); min-width: 200px; padding: 6px; display: flex; flex-direction: column; background: var(--bg-2); border: 1px solid var(--bg-3); border-radius: 10px; box-shadow: 0 8px 24px rgba(0, 0, 0, .35); z-index: 5; }
  .mentions button { display: flex; align-items: center; gap: 8px; text-align: left; padding: 7px 10px; border: 0; border-radius: 7px; background: transparent; color: var(--text-2); font: inherit; }
  .mentions button.on { background: var(--bg-4); color: var(--text); }
  .mentions img { width: 20px; height: 20px; }
  .mentions small { margin-left: auto; padding-left: 12px; color: var(--text-3); font-size: 11px; }
  .emoji-btn { position: absolute; right: 8px; bottom: 8px; width: 32px; height: 32px; border: 0; border-radius: 8px; background: transparent; color: var(--text-3); display: grid; place-items: center; }
  .emoji-btn:hover { color: var(--text); }
  .replying { display: flex; align-items: center; gap: 6px; padding: 8px 14px 0; font-size: 12px; color: var(--text-3); }
  .replying strong { color: var(--text-2); }
  .replying button { margin-left: auto; width: 22px; height: 22px; border: 0; border-radius: 6px; background: transparent; color: var(--text-3); display: grid; place-items: center; }
  /* grows with what's in it (images included), up to 200px, then scrolls */
  .input { width: 100%; box-sizing: border-box; min-height: 48px; max-height: 200px; overflow-y: auto; padding: 14px 48px 14px 14px; color: var(--text); outline: none; white-space: pre-wrap; overflow-wrap: anywhere; line-height: 20px; cursor: text; }
  /* the alt text after the slash keeps the placeholder out of the box's accessible content; the
     line before it is for engines without that syntax (they drop the second line) */
  .input.empty::before { content: attr(data-placeholder); content: attr(data-placeholder) / ''; color: var(--text-3); pointer-events: none; }
  .input :global(img.emoji) { width: 22px; height: 22px; vertical-align: -5px; margin: 0 1px; object-fit: contain; }
</style>
