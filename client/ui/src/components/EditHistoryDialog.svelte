<script lang="ts">
  // Edit history (#301): "(edited)" on a message opens its versions, newest first (the current one at the top), each
  // with when it was written. Plain text, as typed. Anyone who can see the message can open it. Placed by its anchor,
  // as the time picker; Escape or a press outside closes it.
  import { onMount, tick, untrack } from 'svelte'
  import { clickOutside } from '../lib/actions'
  import { popoverStyle } from '../lib/reactions'
  import { api, errorText } from '../lib/tauri'
  import type { MessageVersion } from '../lib/protocol/MessageVersion'
  import Loading from './Loading.svelte'

  let { anchor, messageId, onClose }: {
    anchor: HTMLElement
    messageId: string
    onClose: () => void
  } = $props()

  const SIZE = { width: 340, height: 320 }
  const style = untrack(() => popoverStyle(anchor.getBoundingClientRect(), SIZE, { width: window.innerWidth, height: window.innerHeight }))
  let versions = $state<MessageVersion[] | null>(null)
  let error = $state('')
  let el = $state<HTMLElement>()

  onMount(() => {
    void tick().then(() => el?.focus())
    api.messageEdits(untrack(() => messageId)).then((v) => (versions = v), (e) => (error = errorText(e)))
  })

  const when = (iso: string) => new Date(iso).toLocaleString([], { dateStyle: 'medium', timeStyle: 'short' })

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
</script>

<div bind:this={el} class="history menu-surface pop-in" role="dialog" aria-label="Edit history" tabindex="-1" use:clickOutside={onClose} onkeydown={key} {style}>
  <div class="head">
    <h3 class="menu-label">Edit history</h3>
    <button type="button" class="btn btn-quiet sm" onclick={close}>Close</button>
  </div>
  {#if error}
    <p class="error" role="alert">{error}</p>
  {:else if !versions}
    <Loading />
  {:else}
    <ol class="list">
      {#each versions as v, i (i)}
        <li class="version" class:current={i === 0}>
          <span class="when"><time datetime={v.written_at}>{when(v.written_at)}</time>{#if i === 0} · current{:else if i === versions.length - 1} · original{/if}</span>
          <p class="text">{v.content}</p>
        </li>
      {/each}
    </ol>
  {/if}
</div>

<style>
  .history { width: 340px; max-width: calc(100vw - 12px); max-height: 320px; padding: 12px; gap: 8px; color: var(--text); outline: none; --pop-origin: top left; }
  .head { display: flex; align-items: center; gap: 8px; }
  h3 { flex: 1; margin: 0; }
  .sm { height: 24px; padding: 0 8px; font-size: 12px; }
  .list { margin: 0; padding: 0; list-style: none; overflow-y: auto; min-height: 0; display: flex; flex-direction: column; gap: 6px; }
  .version { padding: 6px 8px; border-radius: var(--radius-sm); background: var(--bg-inset); }
  .version.current { background: var(--bg-4); }
  .when { font-size: 11px; color: var(--text-3); }
  .text { margin: 2px 0 0; font-size: 13px; color: var(--text-body); white-space: pre-wrap; overflow-wrap: anywhere; }
  .error { margin: 0; color: var(--danger-text); font-size: 12px; }
</style>
