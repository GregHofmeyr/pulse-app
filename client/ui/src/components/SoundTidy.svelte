<script lang="ts">
  // "Tidy up?": a full server's five least-played sounds, each with a Delete button. The play counts
  // come fresh from the server (the ones in the app's state can lag).
  import { onMount } from 'svelte'
  import { modal } from '../lib/actions'
  import type { Sound } from '../lib/protocol/Sound'
  import { tidyCandidates } from '../lib/soundboard'
  import { errorText, soundboardCmd } from '../lib/tauri'

  let { serverId, onClose }: { serverId: string; onClose: () => void } = $props()

  let rows = $state<Sound[] | null>(null)
  let busy = $state<string | null>(null)
  let error = $state('')

  onMount(() => {
    soundboardCmd.list(serverId).then((all) => (rows = tidyCandidates(all)), (e) => { rows = []; error = errorText(e) })
  })

  async function del(s: Sound) {
    if (busy) return
    busy = s.id
    error = ''
    try {
      await soundboardCmd.remove(s.id)
      rows = (rows ?? []).filter((r) => r.id !== s.id)
    } catch (e) {
      error = errorText(e)
    } finally {
      busy = null
    }
  }
</script>

<div class="overlay modal-overlay fade-in">
  <div class="backdrop modal-backdrop" role="presentation" data-backdrop onclick={onClose}></div>
  <div class="modal pop-in" role="dialog" aria-modal="true" aria-label="Tidy up sounds" tabindex="-1" use:modal={onClose}>
    <h1 class="modal-title">Tidy up</h1>
    <p>The least-played sounds here. Delete any you don't need to make room.</p>
    {#if rows === null}
      <p class="hint">Loading…</p>
    {:else}
      <ul>
        {#each rows as s (s.id)}
          <li>
            <span class="emoji" aria-hidden="true">{s.emoji ?? ''}</span>
            <span class="name">{s.name}</span>
            <span class="plays">{s.plays} {s.plays === 1 ? 'play' : 'plays'}</span>
            <button type="button" class="btn btn-secondary" disabled={busy !== null} onclick={() => del(s)}>Delete</button>
          </li>
        {:else}
          <li class="none">Nothing to tidy.</li>
        {/each}
      </ul>
    {/if}
    {#if error}<p class="err" role="alert">{error}</p>{/if}
    <div class="actions"><button type="button" class="btn btn-lg btn-secondary" onclick={onClose}>Done</button></div>
  </div>
</div>

<style>
  .overlay { position: fixed; inset: 0; z-index: 30; }
  .modal { width: min(460px, 100%); }
  p { margin: 0; font-size: 14px; color: var(--text-2); }
  .hint { color: var(--text-3); }
  .err { font-size: 13px; color: var(--danger-text); }
  ul { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
  li { display: flex; align-items: center; gap: 10px; min-width: 0; }
  li.none { color: var(--text-3); font-size: 14px; }
  .emoji { width: 22px; text-align: center; }
  .name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .plays { font-size: 13px; color: var(--text-3); }
  .actions { display: flex; justify-content: flex-end; }
</style>
