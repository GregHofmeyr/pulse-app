<script lang="ts">
  import Icon, { type IconName } from './Icon.svelte'
  import type { Channel } from '../lib/protocol/Channel'
  import { app, clock } from '../lib/store.svelte'
  import { callRoster, channelsFor, isMember } from '../lib/selectors'
  import Avatar from './Avatar.svelte'
  import Name from './Name.svelte'
  import { api, errorText } from '../lib/tauri'
  import { isMuted } from '../lib/conversations'
  import { isCollapsed, toggleSection } from '../lib/sidebarSections.svelte'
  import LiveBadge from './LiveBadge.svelte'
  import ParticipantMenu from './ParticipantMenu.svelte'
  import ChannelMenu from './ChannelMenu.svelte'
  import DeleteChannelDialog from './DeleteChannelDialog.svelte'
  import { menuPoint } from '../lib/participantMenu'
  import { selfVoice, voice, voiceApi } from '../lib/voice.svelte'
  import { hoverPrefetch } from '../lib/hoverPrefetch'

  let {
    serverId,
    activeChannelId,
    voiceChannelId,
    speaking,
    onSelect,
    onJoinVoice,
    onMessage,
    onVoiceSettings,
  }: {
    serverId: string
    activeChannelId: string | null
    voiceChannelId: string | null
    speaking: Set<string>
    onSelect: (id: string) => void
    onJoinVoice: (id: string) => void
    onMessage: (userId: string) => void
    onVoiceSettings: () => void
  } = $props()

  const self = $derived(selfVoice())
  const lists = $derived(channelsFor(app.state, serverId))
  // Who's in voice is only shown to people who've joined the server.
  const member = $derived(isMember(app.state, serverId))

  // Join tokens are fetched when the pointer rests on a voice channel, not while it sweeps past.
  const prefetch = hoverPrefetch((id) => voiceApi.prefetch(id))
  /** Hover and focus both start a prefetch; the channel you're already in needs no token. */
  const restOn = (id: string) => { if (member && id !== voiceChannelId) prefetch.enter(id) }
  const mutedForMe = (userId: string) => !!voice.localMutes[userId] && userId !== app.state.me?.id
  /** A voice row is a button: its label is all a screen reader reads, so it carries the state too. */
  const occupantLabel = (p: { userId: string; name: string; muted: boolean; deafened: boolean }) =>
    [p.name, mutedForMe(p.userId) && 'muted for you', p.deafened ? 'deafened' : p.muted && 'muted'].filter(Boolean).join(', ')
  $effect(() => () => prefetch.cancel())

  // the voice occupant under the pointer: its avatar animates
  let hoveredOccupant = $state<string | null>(null)

  // inline "new channel" field
  let adding = $state<'text' | 'quotes' | 'clips' | 'voice' | null>(null)
  let draft = $state('')
  let addError = $state('')

  function startAdd(kind: 'text' | 'quotes' | 'clips' | 'voice') {
    adding = kind
    draft = ''
    addError = ''
  }
  async function submitAdd(e: KeyboardEvent) {
    if (e.key === 'Escape') adding = null
    if (e.key !== 'Enter' || !adding) return
    const kind = adding
    const name = tidyName(kind, draft)
    if (!name) return
    try {
      const c = await api.createChannel(serverId, kind, name)
      adding = null
      if (kind !== 'voice') onSelect(c.id)
    } catch (err) {
      addError = errorText(err)
    }
  }
  const focus = (el: HTMLInputElement) => el.focus()
  /** Text, quotes and clips names read nicer as lower-case-with-dashes, like Discord; voice keeps its case. */
  const tidyName = (kind: string, name: string) => (kind !== 'voice' ? name.trim().toLowerCase().replace(/\s+/g, '-') : name.trim())

  // a channel row's own menu: right-click, or Shift+F10 / the menu key on the focused row
  let chanMenu = $state<{ channel: Channel; x: number; y: number; opener: HTMLElement } | null>(null)
  let chanError = $state('')
  function openChanMenu(c: Channel, e: MouseEvent | KeyboardEvent, anchor: HTMLElement) {
    chanError = ''
    // outside the server a voice channel has nothing to offer
    chanMenu = c.kind === 'voice' && !member ? null : { channel: c, ...menuPoint(e, anchor), opener: anchor }
  }
  function rowKey(e: KeyboardEvent, c: Channel) {
    if (e.key === 'ContextMenu' || (e.key === 'F10' && e.shiftKey)) {
      e.preventDefault()
      openChanMenu(c, e, e.currentTarget as HTMLElement)
    }
  }

  // renaming a channel happens in its row
  let renaming = $state<string | null>(null)
  let renameValue = $state('')
  let renameError = $state('')
  let renameSaving = false
  function startRename(c: Channel) {
    renaming = c.id
    renameValue = c.name ?? ''
    renameError = ''
  }
  async function saveRename(c: Channel) {
    if (renaming !== c.id || renameSaving) return
    const name = tidyName(c.kind, renameValue)
    if (!name || name === c.name) {
      renaming = null
      return
    }
    renameSaving = true
    try {
      await api.renameChannel(c.id, name)
      if (renaming === c.id) renaming = null
    } catch (err) {
      renameError = errorText(err)
    } finally {
      renameSaving = false
    }
  }
  function renameKey(e: KeyboardEvent, c: Channel) {
    if (e.key === 'Enter') void saveRename(c)
    if (e.key === 'Escape') {
      e.preventDefault()
      renaming = null
    }
  }
  let deleting = $state<Channel | null>(null)

  // the participant menu for someone in a voice channel (right-click, click, or the menu key on the focused row)
  let menu = $state<{ channelId: string; userId: string; x: number; y: number; opener: HTMLElement } | null>(null)
  function openMenu(channelId: string, userId: string, e: MouseEvent | KeyboardEvent, anchor: HTMLElement) {
    // clicking the opener again closes its menu
    if (e.type === 'click' && menu?.opener === anchor) menu = null
    else menu = { channelId, userId, ...menuPoint(e, anchor), opener: anchor }
  }
  function occupantKey(e: KeyboardEvent, channelId: string, p: { userId: string; pending: boolean }) {
    if (e.target !== e.currentTarget || p.pending) return
    if (e.key === 'ContextMenu' || (e.key === 'F10' && e.shiftKey) || e.key === 'Enter' || e.key === ' ') {
      e.preventDefault()
      openMenu(channelId, p.userId, e, e.currentTarget as HTMLElement)
    }
  }
  /** In a collapsed section only these rows stay: the channel you're on, and unread ones. */
  const keep = (c: Channel, kind: string) =>
    !isCollapsed(serverId, kind) || c.id === activeChannelId || ((app.state.reads[c.id]?.unread ?? 0) > 0 && !isMuted(app.state, c, clock.nowIso))
</script>

{#snippet heading(kind: string, text: string)}
  {@const open = !isCollapsed(serverId, kind)}
  <button type="button" class="section" aria-expanded={open} onclick={() => toggleSection(serverId, kind)}>
    <span class="caret" class:shut={!open}><Icon name="chevron" size={11} /></span>{text}
  </button>
{/snippet}

{#snippet renameField(c: Channel, icon: IconName)}
  <div class="row renaming">
    <span class="ico"><Icon name={icon} size={17} /></span>
    <input class="input input-sm grow" use:focus bind:value={renameValue} maxlength="64" aria-label="Rename {c.kind === 'voice' ? c.name : `#${c.name}`}"
      onkeydown={(e) => renameKey(e, c)} onblur={() => void saveRename(c)} />
  </div>
  {#if renameError}<small class="err" role="alert">{renameError}</small>{/if}
{/snippet}

{#snippet channelRow(c: Channel, icon: IconName)}
  {@const read = app.state.reads[c.id]}
  {@const muted = isMuted(app.state, c, clock.nowIso)}
  {#if renaming === c.id}
    {@render renameField(c, icon)}
  {:else}
  <button class="row" class:active={c.id === activeChannelId} class:unread={(read?.unread ?? 0) > 0 && !muted} class:muted aria-haspopup="menu"
    onclick={() => onSelect(c.id)}
    oncontextmenu={(e) => { e.preventDefault(); openChanMenu(c, e, e.currentTarget) }} onkeydown={(e) => rowKey(e, c)}>
    <span class="ico"><Icon name={icon} size={17} /></span><span class="grow">{c.name}</span>
    {#if muted}<span class="mic"><Icon name="bellOff" size={13} /></span>{/if}
    {#if read?.mentions}<span class="pill">{read.mentions}</span>{/if}
  </button>
  {/if}
{/snippet}
<div class="list">
  <div class="label">{@render heading('text', 'TEXT CHANNELS')}
    {#if member}<button class="add" aria-label="Create text channel" onclick={() => startAdd('text')}><Icon name="plus" size={14} /></button>{/if}
  </div>
  {#if adding === 'text'}
    <input class="input input-sm new" use:focus bind:value={draft} onkeydown={submitAdd} onblur={() => (adding = null)} placeholder="new-channel" maxlength="64" aria-label="New text channel name" />
    {#if addError}<small class="err">{addError}</small>{/if}
  {/if}
  {#each lists.text as c (c.id)}{#if keep(c, 'text')}{@render channelRow(c, "hash")}{/if}{/each}

  <div class="label voice">{@render heading('quotes', 'QUOTES')}
    {#if member}<button class="add" aria-label="Create quotes channel" onclick={() => startAdd('quotes')}><Icon name="quote" size={14} /></button>{/if}
  </div>
  {#if adding === 'quotes'}
    <input class="input input-sm new" use:focus bind:value={draft} onkeydown={submitAdd} onblur={() => (adding = null)} placeholder="new-quotes" maxlength="64" aria-label="New quotes channel name" />
    {#if addError}<small class="err">{addError}</small>{/if}
  {/if}
  {#each lists.quotes as c (c.id)}{#if keep(c, 'quotes')}{@render channelRow(c, "quote")}{/if}{/each}

  <div class="label voice">{@render heading('clips', 'CLIPS')}
    {#if member}<button class="add" aria-label="Create clips channel" onclick={() => startAdd('clips')}><Icon name="film" size={14} /></button>{/if}
  </div>
  {#if adding === 'clips'}
    <input class="input input-sm new" use:focus bind:value={draft} onkeydown={submitAdd} onblur={() => (adding = null)} placeholder="new-clips" maxlength="64" aria-label="New clips channel name" />
    {#if addError}<small class="err">{addError}</small>{/if}
  {/if}
  {#each lists.clips as c (c.id)}{#if keep(c, 'clips')}{@render channelRow(c, "film")}{/if}{/each}

  <div class="label voice">{@render heading('voice', 'VOICE CHANNELS')}
    {#if member}<button class="add" aria-label="Create voice channel" onclick={() => startAdd('voice')}><Icon name="plus" size={14} /></button>{/if}
  </div>
  {#if adding === 'voice'}
    <input class="input input-sm new" use:focus bind:value={draft} onkeydown={submitAdd} onblur={() => (adding = null)} placeholder="Channel name" maxlength="64" aria-label="New voice channel name" />
    {#if addError}<small class="err">{addError}</small>{/if}
  {/if}
  {#each lists.voice as c (c.id)}
    {#if !isCollapsed(serverId, 'voice') || c.id === voiceChannelId}
    {@const people = member ? callRoster(app.state, serverId, c.id, self) : []}
    {#if renaming === c.id}
      {@render renameField(c, 'speaker')}
    {:else}
    <button class="row" class:active={c.id === activeChannelId} class:live={c.id === voiceChannelId} aria-haspopup={member ? 'menu' : undefined}
      data-tip={c.id === voiceChannelId ? 'You’re here' : `Join ${c.name}`}
      onmouseenter={() => restOn(c.id)} onfocus={() => restOn(c.id)}
      onmouseleave={() => prefetch.leave(c.id)} onblur={() => prefetch.leave(c.id)}
      onclick={() => { prefetch.cancel(); onJoinVoice(c.id) }} ondblclick={() => onSelect(c.id)}
      oncontextmenu={(e) => { e.preventDefault(); openChanMenu(c, e, e.currentTarget) }} onkeydown={(e) => rowKey(e, c)}>
      <span class="ico"><Icon name="speaker" size={17} /></span><span class="grow">{c.name}</span>
      {#if people.length}<span class="count">{people.length}</span>{/if}
    </button>
    {/if}
    {#each people as p (p.userId)}
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <div class="occupant" class:pending={p.pending} role="button" tabindex="0" data-menu-toggle aria-haspopup="menu" aria-label={occupantLabel(p)} data-user={p.userId}
        oncontextmenu={(e) => { e.preventDefault(); if (!p.pending) openMenu(c.id, p.userId, e, e.currentTarget) }}
        onclick={(e) => { if (!p.pending) openMenu(c.id, p.userId, e, e.currentTarget) }}
        onkeydown={(e) => occupantKey(e, c.id, p)}
        onpointerenter={() => (hoveredOccupant = p.userId)} onpointerleave={() => { if (hoveredOccupant === p.userId) hoveredOccupant = null }}>
        <span class="ring" class:speaking={speaking.has(p.userId)}><Avatar id={p.userId} name={p.name} size={22} hovered={() => hoveredOccupant === p.userId} /></span>
        <span class="grow"><Name userId={p.userId} {serverId} interactive={false} /></span>
        {#if p.pending}
          <span class="joining">Joining…</span><span class="spinner" role="progressbar" aria-label="Joining"></span>
        {:else}
          <LiveBadge channelId={c.id} userId={p.userId} />
          <!-- the row's label says these (a button's content isn't read): the icons are decoration -->
          {#if mutedForMe(p.userId)}<span class="flag for-me" data-tip="Muted for you (they don't know)"><Icon name="speakerOff" size={14} /></span>{/if}
          {#if p.deafened}<span class="flag" data-tip="Deafened"><Icon name="headphonesOff" size={14} /></span>
          {:else if p.muted}<span class="flag" data-tip="Muted"><Icon name="micOff" size={14} /></span>{/if}
        {/if}
      </div>
    {/each}
    {/if}
  {/each}
  {#if chanError}<small class="err" role="alert">{chanError}</small>{/if}
</div>
{#if chanMenu}
  {@const c = chanMenu.channel}
  {#key chanMenu}<ChannelMenu channel={c} {member} x={chanMenu.x} y={chanMenu.y} opener={chanMenu.opener} onClose={() => (chanMenu = null)}
    onRename={() => startRename(c)} onDelete={() => (deleting = c)} onJoin={() => onJoinVoice(c.id)} onError={(m) => (chanError = m)} />{/key}
{/if}
{#if deleting}
  <DeleteChannelDialog channel={deleting} onClose={() => (deleting = null)} />
{/if}
{#if menu}
  {#key menu}<ParticipantMenu channelId={menu.channelId} {serverId} userId={menu.userId} x={menu.x} y={menu.y} opener={menu.opener} {onMessage} {onVoiceSettings} onClose={() => (menu = null)} />{/key}
{/if}

<style>
  .list { flex: 1; padding: 8px; display: flex; flex-direction: column; gap: 2px; overflow-y: auto; }
  .label { padding: 12px 10px 6px; font-size: 11px; font-weight: 600; letter-spacing: .06em; color: var(--text-3); }
  .label { display: flex; align-items: center; justify-content: space-between; }
  .label.voice { padding-top: 16px; }
  /* the heading's name hides and shows the section */
  .section { display: flex; align-items: center; gap: 4px; padding: 0; border: 0; background: none; color: inherit; font: inherit; letter-spacing: inherit; cursor: pointer; }
  .section:hover { color: var(--text-2); }
  .caret { display: flex; transition: transform var(--dur) var(--ease); }
  .caret.shut { transform: rotate(-90deg); }
  .add { width: 20px; height: 20px; border: 0; border-radius: 6px; background: transparent; color: var(--text-3); display: grid; place-items: center; padding: 0; }
  .add:hover { color: var(--text); background: var(--bg-2); }
  .new { margin: 2px 4px; }
  .row.renaming { padding-right: 4px; }
  .row.renaming:hover { background: transparent; }
  .err { padding: 0 10px; color: var(--danger-text); font-size: 12px; }
  .row { height: 34px; padding: 0 10px; border: 0; border-radius: 8px; background: transparent; color: var(--text-row); display: flex; align-items: center; gap: 8px; text-align: left; font-size: 14px; }
  .row:hover { background: var(--bg-2); color: var(--text); }
  /* background and colour only: a bolder weight would widen the name */
  .row.active { background: var(--bg-3); color: var(--text); }
  .row.live .ico { color: var(--ok); }
  .ico { color: var(--text-3); display: flex; }
  .grow { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .count { font-size: 11px; color: var(--text-3); }
  .occupant { cursor: default; border-radius: 8px; height: 30px; padding: 0 8px 0 38px; display: flex; align-items: center; gap: 9px; font-size: 13px; color: var(--text-2); }
  .ring { display: flex; flex-shrink: 0; border-radius: 50%; transition: box-shadow .12s; }
  .ring.speaking { box-shadow: 0 0 0 2px var(--bg-1), 0 0 0 4px var(--ok); }
  .occupant:hover { background: var(--bg-2); }
  .occupant.pending { opacity: .6; }
  .joining { font-size: 11px; color: var(--text-2); }
  .spinner { width: 12px; height: 12px; border-radius: 50%; border: 2px solid var(--text-3); border-top-color: transparent; animation: spin .8s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
  @media (prefers-reduced-motion: reduce) { .spinner { animation-duration: 2.4s; } }
  .flag { color: var(--danger-text); display: flex; }
  .flag.for-me { color: var(--warn); }
  .row.unread { color: var(--text); font-weight: 600; }
  .row.muted { opacity: .5; }
  .mic { color: var(--text-3); display: grid; }
  .pill { min-width: 16px; height: 16px; padding: 0 4px; border-radius: 8px; background: var(--danger); color: var(--on-danger); font-size: 10px; font-weight: 700; display: grid; place-items: center; }
</style>
