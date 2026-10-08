<script lang="ts" module>
  // Unsent text per channel, kept while the app runs (switching channels doesn't lose it).
  const drafts = new Map<string, string>()
</script>

<script lang="ts">
  import { untrack } from 'svelte'
  import Icon from './Icon.svelte'
  import { typingThrottle } from '../lib/markdown'
  import { applyMention, mentionQuery, suggest, type Mentionable } from '../lib/mentions'

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
  let box: HTMLTextAreaElement

  // @-mention popup: open while the caret sits right after an @token that matches someone.
  let caret = $state(0)
  let pick = $state(0)
  const query = $derived(mentionQuery(text.slice(0, caret)))
  const options = $derived(query === null ? [] : suggest(mentionables, query))
  let dismissed = $state<string | null>(null) // Esc closes the popup until the token changes
  const open = $derived(options.length > 0 && dismissed !== query)

  function syncCaret() {
    caret = box?.selectionStart ?? text.length
    pick = 0
  }

  async function choose(name: string) {
    const r = applyMention(text, caret, name)
    text = r.text
    caret = r.caret
    await Promise.resolve()
    box.setSelectionRange(r.caret, r.caret)
    box.focus()
  }

  function key(e: KeyboardEvent) {
    if (open) {
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault()
        pick = (pick + (e.key === 'ArrowDown' ? 1 : options.length - 1)) % options.length
        return
      }
      if (e.key === 'Enter' || e.key === 'Tab') {
        e.preventDefault()
        void choose(options[pick].name)
        return
      }
      if (e.key === 'Escape') {
        e.preventDefault()
        dismissed = query
        return
      }
    }
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault()
      const t = text.trim()
      if (!t) return
      onSend(t)
      text = ''
      lastTyping = null
      return
    }
    if (e.key === 'Escape' && replyingTo) onCancelReply()
  }

  function input() {
    syncCaret()
    dismissed = null
    const now = Date.now()
    if (text.trim() && typingThrottle(lastTyping, now)) {
      lastTyping = now
      onTyping()
    }
    box.style.height = 'auto'
    box.style.height = Math.min(box.scrollHeight, 200) + 'px'
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
  {/if}
  {#if replyingTo}
    <div class="replying">Replying to <strong>{replyingTo}</strong>
      <button aria-label="Cancel reply" onclick={onCancelReply}><Icon name="x" size={13} /></button></div>
  {/if}
  <textarea bind:this={box} bind:value={text} rows="1" {placeholder} aria-label={placeholder} onkeydown={key} oninput={input} onclick={syncCaret} onkeyup={(e) => { if (e.key.startsWith("Arrow") && !open) syncCaret() }} maxlength="4000"></textarea>
</div>

<style>
  .wrap { position: relative; margin: 4px 16px 16px; background: var(--bg-3); border-radius: 12px; }
  .mentions { position: absolute; left: 0; bottom: calc(100% + 6px); min-width: 200px; padding: 6px; display: flex; flex-direction: column; background: var(--bg-2); border: 1px solid var(--bg-3); border-radius: 10px; box-shadow: 0 8px 24px rgba(0, 0, 0, .35); z-index: 5; }
  .mentions button { text-align: left; padding: 7px 10px; border: 0; border-radius: 7px; background: transparent; color: var(--text-2); font: inherit; }
  .mentions button.on { background: var(--bg-4); color: var(--text); }
  .replying { display: flex; align-items: center; gap: 6px; padding: 8px 14px 0; font-size: 12px; color: var(--text-3); }
  .replying strong { color: var(--text-2); }
  .replying button { margin-left: auto; width: 22px; height: 22px; border: 0; border-radius: 6px; background: transparent; color: var(--text-3); display: grid; place-items: center; }
  textarea { width: 100%; box-sizing: border-box; min-height: 48px; max-height: 200px; resize: none; border: 0; background: transparent; padding: 14px; font: inherit; color: var(--text); outline: none; }
</style>
