<script lang="ts">
  // Settings › Privacy › Game activity › "Hide what I'm playing from" (#436): the people who never see what you're
  // playing right now (your "Playing" and its Join; your profile and stats still show to them as to anyone), by the
  // name you see them by, in the order you added them, each with Remove.
  // + Add person opens the people picker; their right-click menu does it too. Kept on your account (every device), and
  // enforced by the server: they see you online with no game, and are never told.
  import Avatar from './Avatar.svelte'
  import Icon from './Icon.svelte'
  import PeoplePicker from './PeoplePicker.svelte'
  import { app } from '../lib/store.svelte'
  import { errorText } from '../lib/tauri'
  import { displayName } from '../lib/selectors'
  import { setActivityHidden } from '../lib/gameInvites.svelte'

  const hidden = $derived(app.state.activityHidden.map((id) => ({ id, name: displayName(app.state, null, id) })))
  const candidates = $derived.by(() => {
    const s = app.state
    const ids = new Set([...Object.values(s.members).flatMap((ms) => ms.map((m) => m.user.id)), ...Object.values(s.dmMembers).flat()])
    ids.delete(s.me?.id ?? '')
    for (const id of s.activityHidden) ids.delete(id)
    return [...ids]
  })
  let picking = $state(false)
  let error = $state('')
  async function run(f: () => Promise<unknown>) {
    error = ''
    try {
      await f()
    } catch (e) {
      error = errorText(e)
    }
  }
  function add(ids: string[]) {
    picking = false
    void run(async () => {
      for (const id of ids) await setActivityHidden(id, true)
    })
  }
</script>

<div class="hidden-games" data-hidden-games>
  <h4 class="games-title">Hide what I'm playing from</h4>
  <p class="hint">Hides what you're playing right now, and your Join: they see you online with no game. Your achievement and Deadlock match posts skip servers they're in. They're never told.</p>
  <button type="button" class="btn btn-secondary add" onclick={() => (picking = true)}><Icon name="plus" size={14} /> Add person</button>
  {#if hidden.length}
    <ul class="settings-list">
      {#each hidden as p (p.id)}
        <li class="settings-list-item">
          <Avatar id={p.id} name={p.name} size={28} />
          <span class="who">{p.name}</span>
          <button type="button" class="btn btn-secondary" aria-label="Show {p.name} what you're playing" onclick={() => void run(() => setActivityHidden(p.id, false))}>Remove</button>
        </li>
      {/each}
    </ul>
  {:else}
    <p class="empty">Nobody. Add someone here, or right-click them and pick "Hide what I'm playing".</p>
  {/if}
  {#if error}<p class="error" role="alert">{error}</p>{/if}
</div>

{#if picking}
  <PeoplePicker title="Hide what I'm playing from" hint="They'll see you online with no game. They're never told." only={candidates} max={Infinity}
    action="Hide" onDone={add} onClose={() => (picking = false)} />
{/if}

<style>
  .hidden-games { display: flex; flex-direction: column; gap: 6px; }
  .games-title { margin: 8px 0 0; font-size: 13px; font-weight: 600; }
  .hint, .empty { margin: 0; color: var(--text-2); font-size: 13px; }
  .empty { color: var(--text-3); }
  .add { align-self: flex-start; }
  .who { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: 600; }
  .error { margin: 0; color: var(--danger-text); font-size: 13px; }
</style>
