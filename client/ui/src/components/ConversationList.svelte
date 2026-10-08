<script lang="ts">
  import Icon from './Icon.svelte'
  import Avatar from './Avatar.svelte'
  import { app } from '../lib/store.svelte'
  import { conversations, previewText } from '../lib/conversations'

  let { activeId, onOpen, onNew }: { activeId: string | null; onOpen: (id: string) => void; onNew: () => void } = $props()

  const list = $derived(conversations(app.state))
  const me = $derived(app.state.me?.id)
  const others = (members: string[]) => members.filter((u) => u !== me)
  const nameOf = (id: string) => app.state.people[id]?.user.username ?? '?'
</script>

<button class="new" onclick={onNew}><Icon name="plus" size={15} /> New message</button>

<div class="list">
  {#each list as c (c.channel.id)}
    {@const people = others(c.members)}
    {@const badge = c.muted ? c.mentions : c.unread}
    <button class="row" class:active={c.channel.id === activeId} class:unread={c.unread > 0 && !c.muted} class:muted={c.muted}
      onclick={() => onOpen(c.channel.id)}>
      {#if c.channel.kind === 'dm' && people[0]}
        <Avatar id={people[0]} name={nameOf(people[0])} size={32} online={app.state.people[people[0]]?.online ?? false} />
      {:else}
        <span class="stack">
          {#each people.slice(0, 2) as u, i (u)}
            <span class="s{i}"><Avatar id={u} name={nameOf(u)} size={21} /></span>
          {/each}
        </span>
      {/if}
      <span class="meta">
        <span class="name">{c.name}</span>
        {#if c.latest}
          {@const p = previewText(app.state, c.latest)}
          <span class="prev" class:sys={p.system}>{p.text}</span>
        {/if}
      </span>
      {#if c.muted}<span class="mute"><Icon name="bellOff" size={13} /></span>{/if}
      {#if badge > 0}<span class="pill">{badge > 99 ? '99+' : badge}</span>{/if}
    </button>
  {:else}
    <p class="empty">No conversations yet. Say hi to someone from Your people.</p>
  {/each}
</div>

<style>
  .new { margin: 4px 8px 8px; height: 36px; border: 0; border-radius: 10px; background: var(--bg-3); color: var(--text-2); font-weight: 600; display: flex; align-items: center; justify-content: center; gap: 6px; }
  .new:hover { color: var(--text); background: var(--bg-4); }
  .list { flex: 1; min-height: 0; overflow-y: auto; padding: 0 6px; display: flex; flex-direction: column; gap: 1px; }
  .row { display: flex; align-items: center; gap: 10px; padding: 7px 8px; border: 0; border-radius: 10px; background: transparent; color: var(--text-2); text-align: left; }
  .row:hover { background: var(--bg-2); }
  .row.active { background: var(--bg-3); color: var(--text); }
  .row.muted { opacity: .5; }
  .meta { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  .name { font-weight: 600; font-size: 14px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .row.unread .name { color: var(--text); }
  .prev { font-size: 12px; color: var(--text-3); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .prev.sys { font-style: italic; }
  .stack { position: relative; width: 32px; height: 32px; flex-shrink: 0; }
  .stack .s0 { position: absolute; left: 0; top: 0; }
  .stack .s1 { position: absolute; left: 11px; top: 11px; border-radius: 50%; box-shadow: 0 0 0 2px var(--bg-1); }
  .mute { color: var(--text-3); display: grid; }
  .pill { min-width: 18px; height: 18px; padding: 0 5px; border-radius: 9px; background: var(--danger); color: #fff; font-size: 11px; font-weight: 700; display: grid; place-items: center; }
  .empty { padding: 12px 10px; font-size: 13px; color: var(--text-3); }
</style>
