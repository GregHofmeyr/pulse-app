<script lang="ts">
  import Icon from './Icon.svelte'
  import { app } from '../lib/store.svelte'
  import { channelsFor, isMember, voiceOccupants } from '../lib/selectors'
  import { avatarColor, initial } from '../lib/avatar'
  import { api, errorText } from '../lib/tauri'
  import { isMuted } from '../lib/conversations'

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
  // Who's in voice is only shown to people who've joined the server.
  const member = $derived(isMember(app.state, serverId))

  // inline "new channel" field
  let adding = $state<'text' | 'voice' | null>(null)
  let draft = $state('')
  let addError = $state('')

  function startAdd(kind: 'text' | 'voice') {
    adding = kind
    draft = ''
    addError = ''
  }
  async function submitAdd(e: KeyboardEvent) {
    if (e.key === 'Escape') adding = null
    if (e.key !== 'Enter' || !adding) return
    const kind = adding
    // text channels read nicer as lower-case-with-dashes, like Discord
    const name = kind === 'text' ? draft.trim().toLowerCase().replace(/\s+/g, '-') : draft.trim()
    if (!name) return
    try {
      const c = await api.createChannel(serverId, kind, name)
      adding = null
      if (kind === 'text') onSelect(c.id)
    } catch (err) {
      addError = errorText(err)
    }
  }
  const focus = (el: HTMLInputElement) => el.focus()
</script>

<div class="list">
  <div class="label">TEXT CHANNELS
    {#if member}<button class="add" aria-label="Create text channel" onclick={() => startAdd('text')}><Icon name="plus" size={14} /></button>{/if}
  </div>
  {#if adding === 'text'}
    <input class="new" use:focus bind:value={draft} onkeydown={submitAdd} onblur={() => (adding = null)} placeholder="new-channel" maxlength="64" aria-label="New text channel name" />
    {#if addError}<small class="err">{addError}</small>{/if}
  {/if}
  {#each lists.text as c (c.id)}
    {@const read = app.state.reads[c.id]}
    {@const muted = isMuted(app.state, c, new Date().toISOString())}
    <button class="row" class:active={c.id === activeChannelId} class:unread={(read?.unread ?? 0) > 0 && !muted} class:muted
      onclick={() => onSelect(c.id)}>
      <span class="ico"><Icon name="hash" size={17} /></span><span class="grow">{c.name}</span>
      {#if muted}<span class="mic"><Icon name="bellOff" size={13} /></span>{/if}
      {#if read?.mentions}<span class="pill">{read.mentions}</span>{/if}
    </button>
  {/each}

  <div class="label voice">VOICE CHANNELS
    {#if member}<button class="add" aria-label="Create voice channel" onclick={() => startAdd('voice')}><Icon name="plus" size={14} /></button>{/if}
  </div>
  {#if adding === 'voice'}
    <input class="new" use:focus bind:value={draft} onkeydown={submitAdd} onblur={() => (adding = null)} placeholder="Channel name" maxlength="64" aria-label="New voice channel name" />
    {#if addError}<small class="err">{addError}</small>{/if}
  {/if}
  {#each lists.voice as c (c.id)}
    {@const people = member ? voiceOccupants(app.state, serverId, c.id) : []}
    <button class="row" class:active={c.id === activeChannelId} class:live={c.id === voiceChannelId}
      data-tip={c.id === voiceChannelId ? 'You’re here' : `Join ${c.name}`}
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
  .label { display: flex; align-items: center; justify-content: space-between; }
  .label.voice { padding-top: 16px; }
  .add { width: 20px; height: 20px; border: 0; border-radius: 6px; background: transparent; color: var(--text-3); display: grid; place-items: center; padding: 0; }
  .add:hover { color: var(--text); background: var(--bg-2); }
  .new { height: 32px; margin: 2px 4px; padding: 0 10px; border-radius: 8px; border: 1px solid var(--accent); background: var(--bg-2); font-size: 14px; }
  .err { padding: 0 10px; color: #f2616b; font-size: 12px; }
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
  .row.unread { color: var(--text); font-weight: 600; }
  .row.muted { opacity: .5; }
  .mic { color: var(--text-3); display: grid; }
  .pill { min-width: 16px; height: 16px; padding: 0 4px; border-radius: 8px; background: var(--danger); color: #fff; font-size: 10px; font-weight: 700; display: grid; place-items: center; }
</style>
