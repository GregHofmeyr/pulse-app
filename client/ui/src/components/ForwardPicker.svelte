<script lang="ts">
  // Forward a message (#302): from a message's menu, a picker of the chats you can post in (servers' text and voice
  // chats, your DMs and groups), recent ones first, with a search; an optional comment; then Forward. Placed by its
  // anchor, as the time picker. Enter in the comment forwards; Escape or a press outside closes it. The server checks
  // again that you can see the message and post where it goes.
  import { onMount, tick, untrack } from 'svelte'
  import { clickOutside } from '../lib/actions'
  import { forwardList, forwardTargets, targetLabel } from '../lib/forward'
  import { loadRecents, type Place } from '../lib/quickSwitch'
  import { popoverStyle } from '../lib/reactions'
  import { app, ui } from '../lib/store.svelte'
  import { api, errorText } from '../lib/tauri'
  import type { Message } from '../lib/protocol/Message'

  let { anchor, message, author, onClose }: {
    anchor: HTMLElement
    message: Message
    /** Who said it, by the name that applies where it was said. */
    author: string
    onClose: () => void
  } = $props()

  const SIZE = { width: 360, height: 460 }
  const style = untrack(() => popoverStyle(anchor.getBoundingClientRect(), SIZE, { width: window.innerWidth, height: window.innerHeight }))
  const recents = loadRecents()
  // one per picker: a double press (or a retry after a lost answer) forwards once
  const nonce = crypto.randomUUID()
  let query = $state('')
  let comment = $state('')
  let chosen = $state<Place | null>(null)
  let busy = $state(false)
  let error = $state('')
  let search: HTMLInputElement | undefined = $state()
  const targets = $derived(forwardTargets(app.state))
  const shown = $derived(forwardList(targets, query, recents))
  const preview = $derived.by(() => {
    const f = message.forwarded
    const text = (f ? f.content : message.caption_is_fallback ? '' : message.content).replace(/\s+/g, ' ').trim()
    const files = message.attachments?.length ?? 0
    return text || (files ? `${files} file${files === 1 ? '' : 's'}` : 'a message')
  })
  onMount(() => void tick().then(() => search?.focus()))

  async function go() {
    if (!chosen || busy) return
    busy = true
    error = ''
    const to = chosen
    try {
      await api.forwardMessage(message.id, to.id, comment.trim(), nonce)
      ui.notice = `Forwarded to ${to.kind === 'text' ? `#${to.name}` : to.name}`
      onClose()
    } catch (e) {
      error = errorText(e)
    } finally {
      busy = false
    }
  }
  function key(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.preventDefault()
      e.stopPropagation()
      close()
    }
  }
  function close() {
    if (anchor.isConnected) anchor.focus()
    onClose()
  }
  function searchKey(e: KeyboardEvent) {
    // Enter picks the first match
    if (e.key === 'Enter' && !e.isComposing && shown.length) {
      e.preventDefault()
      chosen = shown[0]
    }
  }
  function commentKey(e: KeyboardEvent) {
    if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
      e.preventDefault()
      void go()
    }
  }
</script>

<div class="forward menu-surface pop-in" role="dialog" aria-label="Forward message" tabindex="-1" use:clickOutside={onClose} onkeydown={key} {style}>
  <h3 class="menu-label">Forward message</h3>
  <p class="preview"><strong>{author}</strong> {preview}</p>
  <input bind:this={search} class="input input-sm" type="search" placeholder="Search chats" aria-label="Search chats" bind:value={query} onkeydown={searchKey} />
  <div class="list" role="listbox" aria-label="Chats">
    {#each shown as p (p.key)}
      <button type="button" class="target" class:on={chosen?.key === p.key} role="option" aria-selected={chosen?.key === p.key} onclick={() => (chosen = p)}>
        <span class="name">{targetLabel(p)}</span>
        <span class="context">{p.kind === 'dm' ? 'Direct message' : (p.context ?? '')}</span>
      </button>
    {:else}
      <p class="none">{targets.length ? 'No chats match that.' : 'No chats to forward to.'}</p>
    {/each}
  </div>
  <textarea class="input input-sm comment" rows="2" placeholder="Add a comment (optional)" aria-label="Comment" bind:value={comment} onkeydown={commentKey}></textarea>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  <div class="foot">
    <button type="button" class="btn btn-quiet" onclick={close}>Cancel</button>
    <button type="button" class="btn btn-primary" disabled={!chosen || busy} onclick={() => void go()}>{chosen ? `Forward to ${targetLabel(chosen)}` : 'Forward'}</button>
  </div>
</div>

<style>
  .forward { width: 360px; max-width: calc(100vw - 12px); max-height: 460px; padding: 12px; gap: 8px; color: var(--text); outline: none; --pop-origin: top left; }
  h3 { margin: 0; }
  .preview { margin: 0; font-size: 13px; color: var(--text-2); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .preview strong { color: var(--text); }
  .list { flex: 1; min-height: 80px; overflow-y: auto; display: flex; flex-direction: column; gap: 2px; }
  .target { display: flex; align-items: baseline; gap: 8px; width: 100%; padding: 6px 8px; border: 0; border-radius: var(--radius-sm); background: transparent; color: inherit; font: inherit; text-align: left; }
  .target:hover, .target:focus-visible { background: var(--bg-4); }
  .target.on { background: var(--accent-soft); box-shadow: inset 2px 0 0 var(--focus-ring); }
  .name { font-size: 13px; font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .context { flex: 1; min-width: 0; font-size: 12px; color: var(--text-3); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .comment { width: 100%; box-sizing: border-box; resize: vertical; font: inherit; }
  .none { margin: 6px 8px; color: var(--text-3); font-size: 13px; }
  .error { margin: 0; color: var(--danger-text); font-size: 12px; }
  .foot { display: flex; justify-content: flex-end; gap: 8px; }
  .foot .btn { height: 30px; padding: 0 12px; font-size: 13px; max-width: 220px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
