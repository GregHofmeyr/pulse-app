<script lang="ts">
  import Icon from './Icon.svelte'
  import Avatar from './Avatar.svelte'
  import { app } from '../lib/store.svelte'

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
  const me = $derived(app.state.me?.id)
  const nameOf = (id: string) => app.state.people[id]?.user.username ?? '?'
  const total = $derived(1 + fixed.length + picked.length)
  const full = $derived(total >= max)
  const people = $derived(
    Object.values(app.state.people)
      .filter((p) => p.user.id !== me && !fixed.includes(p.user.id))
      .filter((p) => p.user.username.toLowerCase().includes(q.trim().toLowerCase()))
      .sort((a, b) => a.user.username.localeCompare(b.user.username)),
  )

  function toggle(id: string) {
    if (picked.includes(id)) picked = picked.filter((x) => x !== id)
    else if (!full) picked = [...picked, id]
  }
</script>

<div class="overlay">
  <div class="backdrop" role="presentation" onclick={onClose}></div>
  <div class="dialog" role="dialog" aria-label={title}>
    <h2>{title}</h2>
    {#if hint}<p class="hint">{hint}</p>{/if}
    {#if fixed.length || picked.length}
      <div class="chips">
        {#each fixed as id (id)}<span class="chip">{nameOf(id)}</span>{/each}
        {#each picked as id (id)}
          <button class="chip" aria-label="Remove {nameOf(id)}" onclick={() => toggle(id)}>{nameOf(id)} <Icon name="x" size={11} /></button>
        {/each}
      </div>
    {/if}
    <!-- svelte-ignore a11y_autofocus -->
    <input class="search" placeholder="Search people" bind:value={q} autofocus />
    <div class="list">
      {#each people as p (p.user.id)}
        {@const isIn = disabled.includes(p.user.id)}
        {@const sel = picked.includes(p.user.id)}
        <button class="pick" class:sel disabled={isIn || (full && !sel)} onclick={() => toggle(p.user.id)}>
          <Avatar id={p.user.id} name={p.user.username} size={30} online={p.online} />
          <span class="nm">{p.user.username}{#if isIn}<small> · already here</small>{/if}</span>
          <span class="box" aria-hidden="true"></span>
        </button>
      {:else}
        <p class="hint">No one matches “{q}”.</p>
      {/each}
    </div>
    <div class="foot">
      <span class="hint">{total} of {max} people</span>
      <button class="ghost" onclick={onClose}>Cancel</button>
      <button class="primary" disabled={!picked.length} onclick={() => onDone(picked)}>{action}</button>
    </div>
  </div>
</div>

<style>
  .overlay { position: fixed; inset: 0; z-index: 30; display: grid; place-items: center; padding: 32px 16px; }
  .backdrop { position: absolute; inset: 0; background: rgba(10, 11, 13, .6); }
  .dialog { position: relative; width: min(420px, calc(100vw - 32px)); max-height: calc(100vh - 64px); display: flex; flex-direction: column; gap: 10px; padding: 20px; background: var(--bg-1); border: 1px solid var(--bg-3); border-radius: 16px; }
  h2 { margin: 0; font-size: 17px; }
  .hint { margin: 0; font-size: 12px; color: var(--text-3); }
  .chips { display: flex; flex-wrap: wrap; gap: 6px; }
  .chip { padding: 4px 10px; border: 0; border-radius: 20px; background: var(--bg-3); color: var(--text); font-size: 12px; display: inline-flex; align-items: center; gap: 4px; }
  .search { height: 36px; padding: 0 12px; border: 0; border-radius: 10px; background: var(--bg-2); color: var(--text); font: inherit; outline: none; }
  .list { overflow-y: auto; min-height: 120px; max-height: 320px; display: flex; flex-direction: column; gap: 2px; }
  .pick { display: flex; align-items: center; gap: 10px; padding: 6px 8px; border: 0; border-radius: 10px; background: transparent; color: var(--text-2); text-align: left; }
  .pick:hover:not(:disabled) { background: var(--bg-2); }
  .pick.sel { background: var(--bg-3); color: var(--text); }
  .pick:disabled { opacity: .45; }
  .nm { flex: 1; font-weight: 500; }
  .nm small { color: var(--text-3); font-weight: 400; }
  .box { width: 16px; height: 16px; border-radius: 5px; border: 2px solid var(--bg-4); }
  .pick.sel .box { background: var(--accent); border-color: var(--accent); }
  .foot { display: flex; align-items: center; gap: 8px; margin-top: 4px; }
  .foot .hint { margin-right: auto; }
  .ghost { height: 36px; padding: 0 14px; border: 0; border-radius: 10px; background: transparent; color: var(--text-2); font-weight: 600; }
  .primary { height: 36px; padding: 0 16px; border: 0; border-radius: 10px; background: var(--accent); color: var(--on-accent); font-weight: 700; }
  .primary:disabled { opacity: .45; }
</style>
