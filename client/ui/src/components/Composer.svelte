<script lang="ts">
  import Icon from './Icon.svelte'
  import { typingThrottle } from '../lib/markdown'

  let {
    placeholder,
    replyingTo,
    onCancelReply,
    onSend,
    onTyping,
  }: {
    placeholder: string
    replyingTo: string | null
    onCancelReply: () => void
    onSend: (text: string) => void
    onTyping: () => void
  } = $props()

  let text = $state('')
  let lastTyping: number | null = null
  let box: HTMLTextAreaElement

  function key(e: KeyboardEvent) {
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
  {#if replyingTo}
    <div class="replying">Replying to <strong>{replyingTo}</strong>
      <button aria-label="Cancel reply" onclick={onCancelReply}><Icon name="x" size={13} /></button></div>
  {/if}
  <textarea bind:this={box} bind:value={text} rows="1" {placeholder} aria-label={placeholder} onkeydown={key} oninput={input} maxlength="4000"></textarea>
</div>

<style>
  .wrap { margin: 4px 16px 16px; background: var(--bg-3); border-radius: 12px; }
  .replying { display: flex; align-items: center; gap: 6px; padding: 8px 14px 0; font-size: 12px; color: var(--text-3); }
  .replying strong { color: var(--text-2); }
  .replying button { margin-left: auto; width: 22px; height: 22px; border: 0; border-radius: 6px; background: transparent; color: var(--text-3); display: grid; place-items: center; }
  textarea { width: 100%; box-sizing: border-box; min-height: 48px; max-height: 200px; resize: none; border: 0; background: transparent; padding: 14px; font: inherit; color: var(--text); outline: none; }
</style>
