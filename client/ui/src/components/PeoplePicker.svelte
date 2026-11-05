<script lang="ts">
  import Icon from './Icon.svelte'
  import { modal } from '../lib/actions'
  import Avatar from './Avatar.svelte'
  import Name from './Name.svelte'
  import { app, clock } from '../lib/store.svelte'
  import { presenceOf } from '../lib/presence'
  import { displayName, namesOf } from '../lib/selectors'

  let {
    title,
    hint = '',
    fixed = [],
    disabled = [],
    max = 10,
    action,
    onDone,
    onClose,
  }: {
    title: string
    hint?: string
    /** Already in (shown as chips, can't be removed), excluding you. */
    fixed?: string[]
    /** Shown dimmed as "already here". */
    disabled?: string[]
    /** Total people including you. */
    max?: number
    action: string
    onDone: (ids: string[]) => void
    onClose: () => void
  } = $props()

  let q = $state('')
  let picked = $state<string[]>([])
  // the row under the pointer: its avatar animates
  let hovered = $state<string | null>(null)
  const me = $derived(app.state.me?.id)
  const nameOf = (id: string) => (app.state.people[id] ? displayName(app.state, null, id) : '?')
  const total = $derived(1 + fixed.length + picked.length)
  const full = $derived(total >= max)
  const people = $derived(
    Object.values(app.state.people)
      .filter((p) => p.user.id !== me && !fixed.includes(p.user.id))
      // found by any name they have for you (#47); listed by the one you see
      .filter((p) => [p.user.username, ...namesOf(app.state, null, p.user.id)].some((n) => n.toLowerCase().includes(q.trim().toLowerCase())))
      .map((p) => ({ ...p, name: displayName(app.state, null, p.user.id) }))
      .sort((a, b) => a.name.localeCompare(b.name)),
  )

  function toggle(id: string) {
    if (picked.includes(id)) picked = picked.filter((x) => x !== id)
    else if (!full) picked = [...picked, id]
  }
</script>

<div class="overlay modal-overlay fade-in">
  <div class="backdrop modal-backdrop" role="presentation" data-backdrop onclick={onClose}></div>
  <div class="modal pop-in" role="dialog" aria-modal="true" aria-label={title} tabindex="-1" use:modal={onClose}>
    <h2 class="modal-title">{title}</h2>
    {#if hint}<p class="hint">{hint}</p>{/if}
    {#if fixed.length || picked.length}
      <div class="chips">
        {#each fixed as id (id)}<span class="chip"><Avatar {id} name={nameOf(id)} size={16} />{nameOf(id)}</span>{/each}
        {#each picked as id (id)}
          <button class="chip" aria-label="Remove {nameOf(id)}" onclick={() => toggle(id)}><Avatar {id} name={nameOf(id)} size={16} />{nameOf(id)} <Icon name="x" size={11} /></button>
        {/each}
      </div>
    {/if}
    <!-- svelte-ignore a11y_autofocus -->
    <input class="input search" placeholder="Search people" bind:value={q} autofocus />
    <div class="list">
      {#each people as p (p.user.id)}
        {@const isIn = disabled.includes(p.user.id)}
        {@const sel = picked.includes(p.user.id)}
        <!-- the row is the pick button: the name is styled but not a button of its own -->
        <button class="pick" class:sel disabled={isIn || (full && !sel)} onclick={() => toggle(p.user.id)}
          onpointerenter={() => (hovered = p.user.id)} onpointerleave={() => { if (hovered === p.user.id) hovered = null }}>
          <Avatar id={p.user.id} name={p.name} size={30} presence={presenceOf(app.state, p.user.id, clock.nowIso)} hovered={() => hovered === p.user.id} />
          <span class="nm"><Name userId={p.user.id} serverId={null} text={p.name} interactive={false} />{#if isIn}<small> · already here</small>{/if}</span>
          <span class="box" aria-hidden="true"></span>
        </button>
      {:else}
        <p class="hint">No one matches “{q}”.</p>
      {/each}
    </div>
    <div class="foot">
      <span class="hint">{total} of {max} people</span>
      <button class="btn btn-lg btn-secondary" onclick={onClose}>Cancel</button>
      <button class="btn btn-lg btn-primary" disabled={!picked.length} onclick={() => onDone(picked)}>{action}</button>
    </div>
  </div>
</div>

<style>
  .overlay { position: fixed; inset: 0; z-index: 30; padding: 32px 16px; }
  .modal { width: min(420px, calc(100vw - 32px)); max-height: calc(100vh - 64px); gap: 10px; }
  .hint { margin: 0; font-size: 12px; color: var(--text-3); }
  .chips { display: flex; flex-wrap: wrap; gap: 6px; }
  .chip { padding: 3px 10px 3px 3px; border: 0; border-radius: 20px; background: var(--bg-3); color: var(--text); font-size: 12px; display: inline-flex; align-items: center; gap: 4px; }
  .list { overflow-y: auto; min-height: 120px; max-height: 320px; display: flex; flex-direction: column; gap: 2px; }
  .pick { display: flex; align-items: center; gap: 10px; padding: 6px 8px; border: 0; border-radius: var(--radius-md); background: transparent; color: var(--text-2); text-align: left; }
  .pick:hover:not(:disabled) { background: var(--bg-3); }
  .pick.sel { background: var(--bg-4); color: var(--text); }
  .pick:disabled { opacity: .45; }
  .nm { flex: 1; font-weight: 500; }
  .nm small { color: var(--text-3); font-weight: 400; }
  .box { width: 16px; height: 16px; border-radius: 4px; border: 2px solid var(--bg-4); }
  .pick.sel .box { background: var(--accent); border-color: var(--focus-ring); }
  .foot { display: flex; align-items: center; gap: 8px; margin-top: 4px; }
  .foot .hint { margin-right: auto; }
</style>
