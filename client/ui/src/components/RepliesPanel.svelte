<script lang="ts">
  // The replies to one quote or clip (spec §3.6): it is pinned at the top, then a normal chat about it (ThreadChat).
  import { untrack } from 'svelte'
  import Icon from './Icon.svelte'
  import QuoteItem from './QuoteItem.svelte'
  import ClipItem from './ClipItem.svelte'
  import ThreadChat from './ThreadChat.svelte'
  import { app } from '../lib/store.svelte'
  import { overlayShown } from '../lib/escape'
  import { nameOfIn } from '../lib/selectors'
  import { api } from '../lib/tauri'
  import type { Message } from '../lib/protocol/Message'

  let { root, serverId, onClose, onEditQuote = () => {} }: {
    /** The quote or clip (kept current by the caller). */
    root: Message
    serverId: string
    onClose: () => void
    /** The pinned quote's Edit (the channel's quote composer edits it). */
    onEditQuote?: () => void
  } = $props()

  // One panel per quote: the caller remounts it for another one.
  const rootId = untrack(() => root.id)
  const meId = $derived(app.state.me?.id ?? '')
  let panel: HTMLElement

  const nameOf = $derived(nameOfIn(() => app.state, serverId))

  // Escape closes the panel, unless something else takes it: a field outside the panel, or a menu,
  // popover or dialog that's showing (some of them listen on the window after this does).
  const TYPING = 'textarea, select, input, [contenteditable="true"], [contenteditable="plaintext-only"]'
  function key(e: KeyboardEvent) {
    if (e.key !== 'Escape' || e.defaultPrevented) return
    const at = document.activeElement
    if (at && !panel.contains(at) && at.closest(TYPING)) return
    if (overlayShown()) return
    e.preventDefault()
    onClose()
  }
</script>

<svelte:window onkeydown={key} />

<aside class="replies-panel" aria-label="Replies" bind:this={panel}>
  <div class="head">
    <Icon name="reply" size={16} /><span class="title">Replies</span>
    <span class="grow"></span>
    <button class="icon-btn" aria-label="Close replies" data-tip="Close" onclick={onClose}><Icon name="x" size={16} /></button>
  </div>
  <div class="pinned">
    {#if root.clip}
      <ClipItem message={root} {serverId} {meId} {nameOf} compact />
    {:else}
      <QuoteItem message={root} {serverId} {meId} {nameOf} onEdit={onEditQuote} onDelete={() => api.deleteMessage(rootId)} />
    {/if}
  </div>
  <ThreadChat {root} {serverId} placeholder={root.clip ? 'Reply about this clip' : 'Reply about this quote'} />
</aside>

<style>
  .replies-panel { position: absolute; top: 0; right: 0; bottom: 0; width: var(--replies-w, 380px); display: flex; flex-direction: column; min-height: 0; background: var(--bg-1); border-left: 1px solid var(--bg-3); z-index: 3; }
  .head { height: 52px; flex-shrink: 0; padding: 0 12px 0 18px; display: flex; align-items: center; gap: 8px; border-bottom: 1px solid var(--bg-3); color: var(--text-3); }
  .title { color: var(--text); font-size: 15px; font-weight: 600; }
  .grow { flex: 1; }
  /* a long quote scrolls on its own, so the replies always get room */
  .pinned { flex-shrink: 0; max-height: 40%; overflow-y: auto; padding: 8px 0 6px; border-bottom: 1px solid var(--bg-3); }
</style>
