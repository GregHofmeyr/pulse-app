<script lang="ts">
  import { app } from '../lib/store.svelte'
  import { avatarColor, initial } from '../lib/avatar'
  import Icon from './Icon.svelte'

  let { serverId, onMessage }: { serverId: string; onMessage?: (userId: string) => void } = $props()
  const members = $derived(app.state.members[serverId] ?? [])
</script>

<aside class="members" aria-label="Members">
  <div class="label">MEMBERS — {members.length}</div>
  {#each members as m (m.user.id)}
    {@const name = m.nickname ?? m.user.username}
    <div class="row">
      <span class="av" style:background={avatarColor(m.user.id)}>{initial(name)}</span>
      <span class="who">{name}{#if m.nickname}<small>{m.user.username}</small>{/if}</span>
      {#if onMessage && m.user.id !== app.state.me?.id}
        <button class="msg" aria-label="Message {name}" data-tip="Message" onclick={() => onMessage(m.user.id)}><Icon name="message" size={15} /></button>
      {/if}
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
  .msg { margin-left: auto; width: 28px; height: 28px; border: 0; border-radius: 8px; background: var(--bg-3); color: var(--text-2); display: grid; place-items: center; opacity: 0; }
  .row:hover .msg, .msg:focus-visible { opacity: 1; }
  .row:hover { background: var(--bg-2); }
</style>
