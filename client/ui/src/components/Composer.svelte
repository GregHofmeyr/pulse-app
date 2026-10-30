<script lang="ts" module>
  // Unsent text per channel, kept while the app runs (switching channels doesn't lose it).
  const drafts = new Map<string, string>()
</script>

<script lang="ts">
  import { untrack } from 'svelte'
  import Icon from './Icon.svelte'
  import EmojiBox, { type Snap } from './EmojiBox.svelte'
  import EmojiPopover from './EmojiPopover.svelte'
  import { customToken, savedTone, search, twemojiUrl, withSkin, type Std } from '../lib/emoji'
  import { applyEmoji, emojiQuery, insertAt } from '../lib/emojiComplete'
  import { animatedEmoji, stillSrc } from '../lib/animate.svelte'
  import { typingThrottle } from '../lib/markdown'
  import { applyMention, mentionQuery, suggest, type Mentionable } from '../lib/mentions'
  import type { CustomEmoji } from '../lib/protocol/CustomEmoji'
  import { memberServers } from '../lib/reactions'
  import { app } from '../lib/store.svelte'
  import Name from './Name.svelte'

  let {
    placeholder,
    draftKey,
    replyingTo,
    onCancelReply,
    onSend,
    onTyping,
    mentionables = [],
    serverId = null,
  }: {
    placeholder: string
    draftKey: string
    replyingTo: string | null
    onCancelReply: () => void
    onSend: (text: string) => void
    onTyping: () => void
    /** People who can be @mentioned here (autocomplete). */
    mentionables?: Mentionable[]
    /** The server it posts in (null in DMs and groups): suggested names use its role colours. */
    serverId?: string | null
  } = $props()

  // The composer remounts per channel ({#key} in Shell), so reading the key once is intended.
  let text = $state(untrack(() => drafts.get(draftKey) ?? ''))
  $effect(() => {
    if (text) drafts.set(draftKey, text)
    else drafts.delete(draftKey)
  })
  let lastTyping: number | null = null
  // The message box shows custom emoji as images; `text` (what's sent and what drafts keep) stays the truth.
  let box: EmojiBox

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
    caret = box.readSel()?.start ?? text.length
    pick = 0
  }

  /** Replace the text (an emoji or mention picked) and put the caret at `r.caret` (the box reports it).
   *  Without `focus` (a Shift-pick in the picker) focus stays where it is and the caret waits in the box. */
  const setText = (r: Snap, focus = true) => void box.setText(r, focus)

  const choose = (name: string) => setText(applyMention(text, caret, name))
  const chooseEmoji = (e: Std | CustomEmoji) => setText(applyEmoji(text, caret, emojiText(e)))

  // `sel` keeps the box's selection while the picker has focus: insert there.
  // A `keep` pick (Shift) leaves the picker open and the box unfocused, with the caret advanced.
  function pickEmoji(e: Std | CustomEmoji, { keep }: { keep: boolean }) {
    if (!keep) picking = false
    const sel = box.selection()
    void setText(insertAt(text, sel.start, sel.end, emojiText(e)), !keep)
  }

  // after the box's own keys (IME, undo/redo); Shift+Enter not taken here adds a line
  function key(e: KeyboardEvent) {
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
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault()
      const t = text.trim()
      if (!t) return
      onSend(t)
      box.setText({ text: '', caret: 0 }, true, false)
      box.forget()
      lastTyping = null
      return
    }
    if (e.key === 'Escape' && replyingTo) {
      e.preventDefault() // taken: a panel around the box doesn't close too
      onCancelReply()
    }
  }

  function input(now: number) {
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
          onmousedown={(e) => { e.preventDefault(); void choose(o.name) }}><span>@<Name userId={o.id} {serverId} text={o.name} interactive={false} /></span></button>
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
      <button class="icon-btn icon-btn-xs" aria-label="Cancel reply" onclick={onCancelReply}><Icon name="x" size={13} /></button></div>
  {/if}
  <EmojiBox bind:this={box} bind:text class="input" label={placeholder} {placeholder}
    onkeydown={key} oninput={input} onclick={syncCaret} oncaret={(at) => (caret = at)}
    onkeyup={(e) => { if (e.key.startsWith('Arrow') && !shown) syncCaret() }} />
  <button bind:this={emojiBtn} class="emoji-btn" type="button" aria-label="Emoji" data-menu-toggle onclick={() => (picking = !picking)}><Icon name="smile" size={20} /></button>
  {#if picking && emojiBtn}
    <EmojiPopover anchor={emojiBtn} onPick={pickEmoji} onClose={() => (picking = false)} />
  {/if}
</div>

<style>
  .wrap { position: relative; margin: 4px 18px 16px; background: var(--bg-3); border-radius: 12px; }
  .mentions { position: absolute; left: 0; bottom: calc(100% + 6px); min-width: 200px; padding: 6px; display: flex; flex-direction: column; background: var(--bg-2); border: 1px solid var(--bg-3); border-radius: 10px; box-shadow: var(--shadow-pop); z-index: 5; }
  .mentions button { display: flex; align-items: center; gap: 8px; text-align: left; padding: 7px 10px; border: 0; border-radius: 8px; background: transparent; color: var(--text-2); font: inherit; }
  .mentions button.on { background: var(--bg-4); color: var(--text); }
  .mentions img { width: 20px; height: 20px; }
  .mentions small { margin-left: auto; padding-left: 12px; color: var(--text-3); font-size: 11px; }
  .emoji-btn { position: absolute; right: 8px; bottom: 8px; width: 32px; height: 32px; border: 0; border-radius: 8px; background: transparent; color: var(--text-3); display: grid; place-items: center; }
  .emoji-btn:hover { color: var(--text); }
  .replying { display: flex; align-items: center; gap: 6px; padding: 8px 14px 0; font-size: 12px; color: var(--text-3); }
  .replying strong { color: var(--text-2); }
  .replying button { margin-left: auto; }
  /* the composer is bg-3: one step up from it */
  .replying button:hover { background: var(--bg-4); }
  /* grows with what's in it (images included), up to 200px, then scrolls */
  .wrap :global(.input) { min-height: 48px; max-height: 200px; padding: 14px 48px 14px 14px; }
</style>
