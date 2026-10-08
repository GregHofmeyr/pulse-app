<script lang="ts">
  import Icon from './Icon.svelte'
  import Avatar from './Avatar.svelte'
  import { app } from '../lib/store.svelte'
  import { api, errorText } from '../lib/tauri'

  let { channelId }: { channelId: string } = $props()
  const members = $derived(
    (app.state.dmMembers[channelId] ?? [])
      .map((id) => app.state.people[id])
      .filter((p) => !!p)
      .sort((a, b) => a.user.username.localeCompare(b.user.username)),
  )
  let error = $state('')

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
  <div class="label">MEMBERS — {members.length}</div>
  {#each members as p (p.user.id)}
    <div class="row">
      <Avatar id={p.user.id} name={p.user.username} size={32} online={p.online} />
      <span class="who">{p.user.username}{#if p.user.id === app.state.me?.id}<small> (you)</small>{/if}</span>
      {#if p.user.id !== app.state.me?.id}
        <button class="x" aria-label="Remove {p.user.username}" data-tip="Remove from group" onclick={() => remove(p.user.id)}><Icon name="x" size={14} /></button>
      {/if}
    </div>
  {/each}
  {#if error}<p class="err" role="alert">{error}</p>{/if}
</aside>

<style>
  .members { width: 220px; flex-shrink: 0; background: var(--bg-1); border-radius: var(--radius); padding: 8px; overflow-y: auto; }
  .label { padding: 14px 10px 6px; font-size: 11px; font-weight: 600; letter-spacing: .06em; color: var(--text-3); }
  .row { height: 44px; padding: 0 10px; border-radius: 9px; display: flex; align-items: center; gap: 10px; }
  .row:hover { background: var(--bg-2); }
  .who { flex: 1; min-width: 0; font-size: 14px; font-weight: 500; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .who small { color: var(--text-3); font-weight: 400; }
  .x { width: 26px; height: 26px; border: 0; border-radius: 7px; background: var(--bg-4); color: var(--text-2); display: grid; place-items: center; opacity: 0; }
  .row:hover .x, .x:focus-visible { opacity: 1; }
  .err { margin: 8px 10px; font-size: 12px; color: #f2616b; }
</style>
