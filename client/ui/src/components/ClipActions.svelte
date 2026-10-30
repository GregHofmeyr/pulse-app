<script lang="ts">
  // The poster's menu on a clip: Edit (caption and game, in a small dialog) and Delete (asked first).
  // Shown only on your own clips; the server refuses anyone else anyway.
  import { tick } from 'svelte'
  import GamePicker from './GamePicker.svelte'
  import Icon from './Icon.svelte'
  import { clickOutside, modal } from '../lib/actions'
  import { api, errorText } from '../lib/tauri'
  import type { Message } from '../lib/protocol/Message'

  let { message, serverId }: { message: Message; serverId: string } = $props()

  const MAX_CAPTION = 300

  let opener: HTMLButtonElement | undefined = $state()
  let keep: HTMLButtonElement | undefined = $state()
  let menu = $state(false)
  let asking = $state(false)
  let editing = $state(false)
  let caption = $state('')
  let game = $state<string | null>(null)
  let busy = $state(false)
  let error = $state('')

  function closeMenu(e?: Event) {
    menu = false
    if (e instanceof KeyboardEvent) opener?.focus()
  }
  function startEdit() {
    menu = false
    caption = message.content
    game = message.clip?.game_id ?? null
    error = ''
    editing = true
  }
  async function askDelete() {
    menu = false
    error = ''
    asking = true
    await tick()
    keep?.focus() // this copy's Cancel (the same clip can also be pinned in its replies panel)
  }
  async function run(f: () => Promise<unknown>, done: () => void) {
    if (busy) return
    busy = true
    error = ''
    try {
      await f()
      done()
    } catch (e) {
      error = errorText(e)
    } finally {
      busy = false
    }
  }
  // The edit's MessageUpdated (and the delete's MessageDeleted) change the feeds; nothing is applied here.
  const save = () => run(() => api.editClip(message.id, { caption: caption.trim(), game_id: game }), () => (editing = false))
  const remove = () => run(() => api.deleteMessage(message.id), () => (asking = false))
  const cancelEdit = () => {
    if (!busy) editing = false
  }
</script>

<div class="clip-actions">
  <button bind:this={opener} type="button" class="icon-btn icon-btn-sm" aria-label="Clip actions" aria-haspopup="menu" aria-expanded={menu} data-menu-toggle
    onclick={() => (menu = !menu)}><Icon name="dots" size={15} /></button>
  {#if menu}
    <div class="menu" role="menu" use:clickOutside={closeMenu}>
      <button type="button" role="menuitem" onclick={startEdit}><Icon name="pencil" size={14} />Edit</button>
      <button type="button" role="menuitem" class="danger" onclick={() => void askDelete()}><Icon name="trash" size={14} />Delete</button>
    </div>
  {/if}
  {#if asking}
    <div class="ask" role="alertdialog" aria-label="Delete clip" use:clickOutside={() => { if (!busy) asking = false }}>
      <p>Delete this clip? Its replies go too.</p>
      <button bind:this={keep} type="button" class="btn btn-secondary keep" disabled={busy} onclick={() => (asking = false)}>Cancel</button>
      <button type="button" class="btn btn-danger go" disabled={busy} onclick={() => void remove()}>Delete</button>
      {#if error}<p class="err" role="alert">{error}</p>{/if}
    </div>
  {/if}
</div>

{#if editing}
  <div class="overlay modal-overlay">
    <div class="backdrop modal-backdrop" role="presentation" data-backdrop onclick={cancelEdit}></div>
    <div class="modal" role="dialog" aria-modal="true" aria-label="Edit clip" tabindex="-1" use:modal={cancelEdit}>
      <h1 class="modal-title">Edit clip</h1>
      <textarea aria-label="Caption" placeholder="Say something about it (optional)" maxlength={MAX_CAPTION} rows="3" bind:value={caption} disabled={busy}></textarea>
      <GamePicker {serverId} value={game} onChange={(id) => (game = id)} />
      {#if error}<p class="err" role="alert">{error}</p>{/if}
      <div class="foot">
        <button type="button" class="btn btn-lg btn-secondary" disabled={busy} onclick={cancelEdit}>Cancel</button>
        <button type="button" class="btn btn-lg btn-primary" disabled={busy} onclick={() => void save()}>Save</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .clip-actions { position: relative; }
  .menu { position: absolute; z-index: 6; right: 0; top: calc(100% + 4px); min-width: 140px; padding: 4px; display: flex; flex-direction: column; background: var(--bg-3); border: 1px solid var(--line); border-radius: 10px; box-shadow: var(--shadow-pop); }
  .menu button { height: 32px; padding: 0 10px; border: 0; border-radius: 8px; background: none; color: var(--text); display: flex; align-items: center; gap: 8px; font-size: 13px; text-align: left; }
  .menu button:hover, .menu button:focus-visible { background: var(--bg-4); }
  .menu .danger { color: var(--danger); }
  .ask { position: absolute; z-index: 6; right: 0; top: calc(100% + 4px); width: 240px; padding: 10px; display: flex; flex-wrap: wrap; justify-content: flex-end; gap: 6px; background: var(--bg-3); border: 1px solid var(--line); border-radius: 10px; box-shadow: var(--shadow-pop); }
  .ask p { flex-basis: 100%; margin: 0 0 4px; font-size: 13px; color: var(--text); }
  .ask .err { margin: 4px 0 0; color: var(--danger); }
  .overlay { position: fixed; inset: 0; z-index: 30; }
  .modal { width: min(420px, 100%); }
  textarea { resize: vertical; min-height: 64px; padding: 8px 10px; border-radius: 8px; border: 1px solid var(--bg-4); background: var(--bg-1); color: var(--text); font: inherit; }
  .err { margin: 0; font-size: 12px; color: var(--danger); }
  .foot { display: flex; justify-content: flex-end; gap: 8px; }
</style>
