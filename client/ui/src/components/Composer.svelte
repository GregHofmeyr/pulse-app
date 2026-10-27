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
  import { offsetOf, positionAt, renderInto, serialize } from '../lib/composerDom'
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
  /** A Shift-pick moved the caret while focus was in the picker: put it there when the box gets focus back. */
  let caretPending = false
  function focused() {
    if (!caretPending) return
    caretPending = false
    placeCaret(sel.start)
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
    render(untrack(() => text))
    const onSel = () => {
      const r = readSel()
      if (r) sel = r
    }
    document.addEventListener('selectionchange', onSel)
    return () => {
      document.removeEventListener('selectionchange', onSel)
      unwire.forEach((u) => u())
    }
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
  async function setText(r: { text: string; caret: number }, focus = true) {
    text = r.text.slice(0, MAX)
    caret = Math.min(r.caret, text.length)
    sel = { start: caret, end: caret }
    render(text)
    await Promise.resolve()
    caretPending = !focus
    if (!focus) return
    box.focus()
    placeCaret(caret)
  }

  const choose = (name: string) => setText(applyMention(text, caret, name))
  const chooseEmoji = (e: Std | CustomEmoji) => setText(applyEmoji(text, caret, emojiText(e)))

  // The textarea keeps its selection while the picker has focus: insert there.
  // A `keep` pick (Shift) leaves the picker open and the textarea unfocused, with the caret advanced.
  function pickEmoji(e: Std | CustomEmoji, { keep }: { keep: boolean }) {
    if (!keep) picking = false
    void setText(insertAt(text, sel.start, sel.end, emojiText(e)), !keep)
  }

  /** `insert` as typed (no trailing space) over the selection. */
  function insertRaw(insert: string) {
    const r = readSel() ?? sel
    void setText({ text: text.slice(0, r.start) + insert + text.slice(r.end), caret: r.start + insert.length })
  }
  // Only text comes in: a paste or drop of HTML would put markup in the box.
  function paste(e: ClipboardEvent) {
    e.preventDefault()
    const t = e.clipboardData?.getData('text/plain') ?? ''
    if (t) insertRaw(t.replace(/\r\n?/g, '\n'))
  }

  function key(e: KeyboardEvent) {
    if (shown) {
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault()
        pick = (pick + (e.key === 'ArrowDown' ? 1 : shown - 1)) % shown
        return
      }
      if (e.key === 'Enter' || e.key === 'Tab') {
        e.preventDefault()
        void (open ? choose(options[pick].name) : chooseEmoji(emojiOptions[pick]))
        return
      }
      if (e.key === 'Escape') {
        e.preventDefault()
        dismissed = open ? query : ':' + emojiQ
        return
      }
    }
    if (e.key === 'Enter' && e.shiftKey && !e.isComposing) {
      // a newline in the text itself (the browser would wrap lines in elements)
      e.preventDefault()
      insertRaw('\n')
      return
    }
    if (e.key === 'Enter' && !e.isComposing) {
      e.preventDefault()
      const t = text.trim()
      if (!t) return
      onSend(t)
      void setText({ text: '', caret: 0 })
      lastTyping = null
      return
    }
    if (e.key === 'Escape' && replyingTo) onCancelReply()
  }

  function input(e: Event) {
    const next = serialize(box)
    if (next.length > MAX) {
      // over the limit: back to what fits, caret at the end of it
      void setText({ text: next.slice(0, MAX), caret: MAX })
    } else {
      text = next
      // the browser leaves a lone <br> in a box it emptied: clear it, so the placeholder shows
      if (!next && box.childNodes.length && !(e as InputEvent).isComposing) render('')
    }
    syncCaret()
    dismissed = null
    const now = Date.now()
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
  <div bind:this={box} class="input" class:empty={!text} contenteditable="true" role="textbox" tabindex="0" aria-multiline="true" aria-label={placeholder} data-placeholder={placeholder}
    onkeydown={key} oninput={input} onpaste={paste} onfocus={focused} ondrop={(e) => e.preventDefault()} onclick={syncCaret}
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
  .input.empty::before { content: attr(data-placeholder); color: var(--text-3); pointer-events: none; }
  .input :global(img.emoji) { width: 22px; height: 22px; vertical-align: -5px; margin: 0 1px; object-fit: contain; }
</style>
