<script lang="ts">
  import Icon from './Icon.svelte'
  import { app } from '../lib/store.svelte'
  import { channelsFor, voiceOccupants } from '../lib/selectors'
  import { avatarColor, initial } from '../lib/avatar'

  let {
    serverId,
    activeChannelId,
    voiceChannelId,
    speaking,
    onSelect,
    onJoinVoice,
  }: {
    serverId: string
    activeChannelId: string | null
    voiceChannelId: string | null
    speaking: Set<string>
    onSelect: (id: string) => void
    onJoinVoice: (id: string) => void
  } = $props()

  const lists = $derived(channelsFor(app.state, serverId))
</script>

<div class="list">
  <div class="label">TEXT CHANNELS</div>
  {#each lists.text as c (c.id)}
    <button class="row" class:active={c.id === activeChannelId} onclick={() => onSelect(c.id)}>
      <span class="ico"><Icon name="hash" size={17} /></span>{c.name}
    </button>
  {/each}

  <div class="label voice">VOICE CHANNELS</div>
  {#each lists.voice as c (c.id)}
    {@const people = voiceOccupants(app.state, serverId, c.id)}
    <button class="row" class:active={c.id === activeChannelId} class:live={c.id === voiceChannelId}
      onclick={() => onJoinVoice(c.id)} ondblclick={() => onSelect(c.id)}>
      <span class="ico"><Icon name="speaker" size={17} /></span><span class="grow">{c.name}</span>
      {#if people.length}<span class="count">{people.length}</span>{/if}
    </button>
    {#each people as p (p.userId)}
      <div class="occupant">
        <span class="av" class:speaking={speaking.has(p.userId)} style:background={avatarColor(p.userId)}>{initial(p.name)}</span>
        <span class="grow">{p.name}</span>
        {#if p.deafened}<span class="flag"><Icon name="headphonesOff" size={14} label="Deafened" /></span>
        {:else if p.muted}<span class="flag"><Icon name="micOff" size={14} label="Muted" /></span>{/if}
      </div>
    {/each}
  {/each}
</div>

<style>
  .list { flex: 1; padding: 8px; display: flex; flex-direction: column; gap: 2px; overflow-y: auto; }
  .label { padding: 12px 10px 6px; font-size: 11px; font-weight: 600; letter-spacing: .06em; color: var(--text-3); }
  .label.voice { padding-top: 16px; }
  .row { height: 34px; padding: 0 10px; border: 0; border-radius: 8px; background: transparent; color: #9a9eab; display: flex; align-items: center; gap: 8px; text-align: left; font-size: 14px; }
  .row:hover { background: var(--bg-2); color: var(--text); }
  .row.active { background: var(--bg-3); color: #f1f2f5; font-weight: 500; }
  .row.live .ico { color: var(--ok); }
  .ico { color: var(--text-3); display: flex; }
  .grow { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .count { font-size: 11px; color: var(--text-3); }
  .occupant { height: 30px; padding: 0 8px 0 38px; display: flex; align-items: center; gap: 9px; font-size: 13px; color: var(--text-2); }
  .av { width: 22px; height: 22px; border-radius: 50%; color: #fff; font-size: 10px; font-weight: 700; display: grid; place-items: center; flex-shrink: 0; transition: box-shadow .12s; }
  .av.speaking { box-shadow: 0 0 0 2px var(--bg-1), 0 0 0 4px var(--ok); }
  .flag { color: #f2616b; display: flex; }
</style>
