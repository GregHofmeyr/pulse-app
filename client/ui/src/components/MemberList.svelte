<script lang="ts">
  import { app } from '../lib/store.svelte'
  import { avatarColor, initial } from '../lib/avatar'

  let { serverId }: { serverId: string } = $props()
  const members = $derived(app.state.members[serverId] ?? [])
</script>

<aside class="members" aria-label="Members">
  <div class="label">MEMBERS — {members.length}</div>
  {#each members as m (m.user.id)}
    {@const name = m.nickname ?? m.user.username}
    <div class="row">
      <span class="av" style:background={avatarColor(m.user.id)}>{initial(name)}</span>
      <span class="who">{name}{#if m.nickname}<small>{m.user.username}</small>{/if}</span>
    </div>
  {/each}
</aside>

<style>
  .members { width: 240px; flex-shrink: 0; background: var(--bg-1); border-radius: var(--radius); padding: 8px; overflow-y: auto; }
  .label { padding: 14px 10px 6px; font-size: 11px; font-weight: 600; letter-spacing: .06em; color: var(--text-3); }
  .row { height: 44px; padding: 0 10px; border-radius: 9px; display: flex; align-items: center; gap: 10px; }
  .av { width: 32px; height: 32px; border-radius: 50%; color: #fff; font-weight: 700; display: grid; place-items: center; flex-shrink: 0; }
  .who { display: flex; flex-direction: column; font-size: 14px; font-weight: 500; min-width: 0; }
  .who small { font-size: 11px; color: var(--text-3); font-weight: 400; }
</style>
