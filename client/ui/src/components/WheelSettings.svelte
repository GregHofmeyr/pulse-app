<script lang="ts">
  // Settings › Wheels (#158): a server's wheels (Server settings, anyone in it may change them) or your Just me
  // wheels (your own Settings). New wheel, then per wheel Edit (rename, its options with the popover's own editor)
  // and Delete (asks first). Spinning happens in a chat, from the wheel button by the message box.
  import { flushSync, onMount } from 'svelte'
  import Icon from './Icon.svelte'
  import Wheel from './Wheel.svelte'
  import WheelEditor from './WheelEditor.svelte'
  import { errorText, wheelCmd } from '../lib/tauri'
  import { dropWheel, loadMyWheels, loadServerWheels, saveWheel, upsertWheel, wheels } from '../lib/wheels.svelte'
  import { WHEEL_LIMITS } from '../lib/wheel'
  import type { Server } from '../lib/protocol/Server'
  import type { Wheel as SavedWheel } from '../lib/protocol/Wheel'

  let { server = null }: { server?: Server | null } = $props()

  const list = $derived(server ? (wheels.server[server.id] ?? []) : (wheels.mine ?? []))
  let error = $state('')
  let editing = $state<string | null>(null)
  let confirming = $state<string | null>(null)
  let adding = $state(false)
  let newName = $state('')
  let names = $state<Record<string, string>>({})
  /** What's typed in the open edit's add field, not yet added. */
  let optionDraft = $state('')
  const focusOnShow = (el: HTMLElement) => el.focus()

  // Escape (#264) closes the open edit or new wheel, not the whole dialog; asking first ("Discard changes?") when
  // typing would be lost: a name not yet saved, an option not yet added, a new wheel's name. Escape while asking
  // means Keep editing. With neither open the dialog's own Escape closes it.
  type Open = 'new' | string // a wheel's id
  let asking = $state<Open | null>(null)
  let backTo: HTMLElement | null = null
  let pane: HTMLElement | undefined = $state()
  const dirty = (o: Open) => {
    if (o === 'new') return newName.trim() !== ''
    const w = list.find((x) => x.id === o)
    return optionDraft.trim() !== '' || (!!w && names[o] !== undefined && names[o] !== w.name)
  }
  // one edit open at a time, its add field starting empty
  function edit(id: string | null) {
    if (asking !== 'new') asking = null
    optionDraft = ''
    editing = id
  }
  function shut(o: Open) {
    asking = null
    if (o === 'new') {
      adding = false
      newName = ''
    } else {
      delete names[o]
      if (editing === o) edit(null)
    }
    flushSync()
    pane?.querySelector<HTMLElement>(o === 'new' ? 'button.add' : `button[data-edit="${o}"]`)?.focus()
  }
  function keepEditing() {
    asking = null
    flushSync()
    if (backTo?.isConnected) backTo.focus()
  }
  function escape(e: KeyboardEvent) {
    if (e.key !== 'Escape' || e.defaultPrevented || !pane) return
    // only Escape meant for this dialog (focus is kept inside it)
    const at = e.target as Node
    if (at !== document.body && !pane.closest('[role=dialog]')?.contains(at)) return
    if (asking) {
      e.preventDefault()
      return void keepEditing()
    }
    const inNew = at instanceof Element && !!at.closest('form.new')
    // the edit shown (a wheel deleted meanwhile has none)
    const ed = editing && list.some((w) => w.id === editing) ? editing : null
    const o: Open | null = inNew || (adding && !ed) ? 'new' : (ed ?? (adding ? 'new' : null))
    if (!o) return
    e.preventDefault()
    if (!dirty(o)) return void shut(o)
    backTo = pane.contains(document.activeElement) ? (document.activeElement as HTMLElement) : null
    asking = o
    flushSync()
    pane.querySelector<HTMLElement>('[data-keep]')?.focus()
  }

  onMount(() => {
    ;(server ? loadServerWheels(server.id) : loadMyWheels()).catch((e) => (error = errorText(e)))
  })

  const count = (w: SavedWheel) => `${w.options.length} ${w.options.length === 1 ? 'option' : 'options'}`

  async function create(e: SubmitEvent) {
    e.preventDefault()
    const name = newName.trim()
    if (!name) return
    error = ''
    try {
      const w = await wheelCmd.create(server?.id ?? null, name, [])
      upsertWheel(w)
      adding = false
      newName = ''
      edit(w.id)
    } catch (err) {
      error = errorText(err)
    }
  }

  async function save(w: SavedWheel, change: { name?: string; options?: string[] }) {
    error = ''
    error = await saveWheel(w, change)
  }

  // the field shows your typing until you leave it, then the wheel's name again
  function rename(w: SavedWheel) {
    if (asking === w.id) return // asked whether to discard it: not saved by the field losing focus
    const n = (names[w.id] ?? w.name).trim()
    delete names[w.id]
    if (n && n !== w.name) void save(w, { name: n })
  }

  async function remove(w: SavedWheel) {
    error = ''
    try {
      await wheelCmd.remove(w.id)
      dropWheel(w.id, w.server_id)
      confirming = null
    } catch (e) {
      error = errorText(e)
    }
  }
</script>

<svelte:document onkeydown={escape} />

<section class="settings-pane" bind:this={pane}>
  <header>
    <h2>Wheels</h2>
    <span class="count" aria-label="Wheels">{list.length}</span>
  </header>
  <p class="hint">
    {#if server}Anyone in the server can make, edit and delete its wheels, and spin them in its chats.
    {:else}Only you see these. Spin them in any chat with the wheel button by the message box.{/if}
  </p>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  {#if adding}
    <form class="new" onsubmit={create}>
      <input class="input input-sm" bind:value={newName} maxlength={WHEEL_LIMITS.nameChars} placeholder="Name the wheel" aria-label="New wheel's name"
        {@attach focusOnShow} />
      {#if asking === 'new'}
        {@render discard('new')}
      {:else}
        <button type="submit" class="btn btn-primary" disabled={!newName.trim()}>Create</button>
        <button type="button" class="btn btn-secondary" onclick={() => (adding = false)}>Cancel</button>
      {/if}
    </form>
  {:else}
    <button type="button" class="btn btn-secondary add" onclick={() => (adding = true)}><Icon name="plus" size={14} /> New wheel</button>
  {/if}

  {#if list.length === 0}
    <p class="empty">No wheels yet.</p>
  {:else}
    <ul class="settings-list wheel-list">
      {#each list as w (w.id)}
        <li class="settings-list-item">
          <Wheel options={w.options} size={32} bare />
          <div class="meta">
            <span class="name">{w.name}</span>
            <small>{count(w)}</small>
          </div>
          {#if asking === w.id}
            {@render discard(w.id)}
          {:else if confirming === w.id}
            <span class="q">Delete {w.name}?</span>
            <button type="button" class="btn btn-secondary btn-danger-text" onclick={() => remove(w)}>Delete</button>
            <button type="button" class="btn btn-secondary" onclick={() => (confirming = null)}>Cancel</button>
          {:else}
            <button type="button" class="btn btn-secondary" aria-label="Edit {w.name}" data-edit={w.id} aria-expanded={editing === w.id}
              onclick={() => edit(editing === w.id ? null : w.id)}>{editing === w.id ? 'Done' : 'Edit'}</button>
            <button type="button" class="btn btn-secondary" aria-label="Delete {w.name}" onclick={() => (confirming = w.id)}>Delete</button>
          {/if}
        </li>
        {#if editing === w.id}
          <li class="edit">
            <label class="field">
              <span class="menu-label">Name</span>
              <input class="input input-sm" value={names[w.id] ?? w.name} maxlength={WHEEL_LIMITS.nameChars} aria-label="Name of {w.name}"
                oninput={(e) => (names[w.id] = e.currentTarget.value)} onblur={() => rename(w)}
                onkeydown={(e) => { if (e.key === 'Enter') { e.preventDefault(); rename(w) } }} />
            </label>
            <WheelEditor bind:draft={optionDraft} options={w.options} onChange={(options) => save(w, { options })} autofocus={w.options.length === 0} />
          </li>
        {/if}
      {/each}
    </ul>
  {/if}
</section>

{#snippet discard(o: Open)}
  <span class="q" role="alert">Discard changes?</span>
  <button type="button" class="btn btn-secondary btn-danger-text" onclick={() => shut(o)}>Discard</button>
  <button type="button" class="btn btn-secondary" data-keep onclick={keepEditing}>Keep editing</button>
{/snippet}

<style>
  header { display: flex; align-items: baseline; gap: 12px; }
  h2 { margin: 0; font-size: 20px; font-weight: 600; }
  .count { color: var(--text-3); font-size: 13px; }
  .hint { margin: -12px 0 0; color: var(--text-3); font-size: 12px; }
  .error { margin: 0; color: var(--danger-text); font-size: 13px; }
  .empty { margin: 0; color: var(--text-3); }
  .add { align-self: flex-start; }
  .new { display: flex; gap: 6px; align-items: center; }
  .new .input { width: 260px; }
  /* the list is the shared .settings-list (app.css) */
  .meta { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .name { font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  small { color: var(--text-3); }
  .q { font-size: 13px; color: var(--text-2); }
  .edit { display: flex; flex-direction: column; gap: 10px; padding: 12px; margin-top: -4px; border-radius: var(--radius-md); background: var(--bg-2); max-height: 380px; }
  .field { display: flex; flex-direction: column; gap: 4px; max-width: 320px; }
  .field .menu-label { padding: 0; }
  .edit :global(.editor) { min-height: 0; flex: 1; }
</style>
