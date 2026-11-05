<script lang="ts">
  import Icon from './Icon.svelte'
  import Avatar from './Avatar.svelte'
  import Name from './Name.svelte'
  import { app, clock } from '../lib/store.svelte'
  import { byPresence, presenceOf } from '../lib/presence'
  import { api, errorText } from '../lib/tauri'
  import { displayName } from '../lib/selectors'

  let { channelId }: { channelId: string } = $props()
  // by the name you see them by (#47)
  const members = $derived(
    (app.state.dmMembers[channelId] ?? [])
      .map((id) => app.state.people[id])
      .filter((p) => !!p)
      .map((p) => ({ ...p, name: displayName(app.state, null, p.user.id) }))
      .sort((a, b) => a.name.localeCompare(b.name)),
  )
  // Online (DND included) above Offline (#170), as on the server member list (MemberList.svelte)
  const sections = $derived.by(() => {
    const { online, offline } = byPresence(app.state, members, (p) => p.user.id, clock.nowIso)
    return [{ key: 'online', label: 'Online', list: online }, { key: 'offline', label: 'Offline', list: offline }].filter((g) => g.list.length)
  })
  let error = $state('')
  // the row under the pointer: its avatar animates
  let hovered = $state<string | null>(null)

  async function remove(id: string) {
    error = ''
    try {
      await api.removeMember(channelId, id)
    } catch (e) {
      error = errorText(e)
    }
  }
</script>

<aside class="members" aria-label="Group members">
  {#each sections as g (g.key)}
  <div class="label">{g.label} — {g.list.length}</div>
  {#each g.list as p (p.user.id)}
    <div class="row hover-row" class:off={g.key === 'offline'} data-section={g.key} role="presentation" onpointerenter={() => (hovered = p.user.id)} onpointerleave={() => { if (hovered === p.user.id) hovered = null }}>
      <Avatar id={p.user.id} name={p.name} size={32} presence={presenceOf(app.state, p.user.id, clock.nowIso)} interactive serverId={null} hovered={() => hovered === p.user.id} />
      <span class="who"><Name userId={p.user.id} serverId={null} text={p.name} surface="side" />{#if p.user.id === app.state.me?.id}<small> (you)</small>{/if}</span>
      {#if p.user.id !== app.state.me?.id}
        <button class="x" aria-label="Remove {p.name}" data-tip="Remove from group" onclick={() => remove(p.user.id)}><Icon name="x" size={14} /></button>
      {/if}
    </div>
  {/each}
  {/each}
  {#if error}<p class="err" role="alert">{error}</p>{/if}
</aside>

<style>
  .members { width: var(--right-w, 220px); flex-shrink: 0; background: var(--bg-1); border-radius: var(--radius); padding: 8px; overflow-y: auto; }
  .label { padding: 14px 10px 6px; font-size: 11px; font-weight: 600; letter-spacing: .06em; text-transform: uppercase; color: var(--text-3); }
  .row { height: 44px; padding: 0 10px; border-radius: var(--radius-md); display: flex; align-items: center; gap: 10px; }
  .row:hover { background: var(--bg-2); --ring: var(--bg-2); }
  /* offline (#170): dimmed slightly, as on Discord; back to full while hovered */
  .row.off { opacity: .55; }
  .row.off:hover { opacity: 1; }
  /* the status dot (#169): small, cut out of the avatar in the row's own colour (--ring), as on Discord */
  .row :global(.pres) { width: 12px; height: 12px; min-width: 0; min-height: 0; right: -1px; bottom: -1px; }
  .who { flex: 1; min-width: 0; font-size: 14px; font-weight: 500; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .who small { color: var(--text-3); font-weight: 400; }
  /* a row's revealed action: the same as the server member list's Message (#173) */
  .x { width: 28px; height: 28px; flex-shrink: 0; border: 0; border-radius: var(--radius-sm); background: var(--bg-3); color: var(--text-2); display: grid; place-items: center; opacity: 0; }
  .x:hover { background: var(--bg-4); color: var(--text); }
  .row:hover .x, .x:focus-visible { opacity: 1; }
  .err { margin: 8px 10px; font-size: 13px; color: var(--danger-text); }
</style>
