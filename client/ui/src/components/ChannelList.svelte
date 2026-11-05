<script lang="ts">
  // A server's channel list (#54): the uncategorised channels on top (no heading), then each category with its
  // heading. An older server without categories shows today's four kind sections, each with its own +.
  // Structure (the drag action, #55, relies on it): `.list` > `.cat.top` (rows) and one `.cat[data-category=<id>]`
  // per category (legacy sections: `.cat` without data-category), each `.label` heading then its rows `[data-channel]`.
  // Members drag rows and category headings to reorder (lib/drag.ts); legacy sections and non-members don't drag.
  import { tick, untrack } from 'svelte'
  import Icon, { type IconName } from './Icon.svelte'
  import type { Channel } from '../lib/protocol/Channel'
  import type { Category } from '../lib/protocol/Category'
  import { app, clock } from '../lib/store.svelte'
  import { callRoster, isMember, serverVoiceTip, type Occupant } from '../lib/selectors'
  import Avatar from './Avatar.svelte'
  import Name from './Name.svelte'
  import { api, errorText } from '../lib/tauri'
  import { isMuted } from '../lib/conversations'
  import { isCollapsed, setCollapsed, toggleSection } from '../lib/sidebarSections.svelte'
  import { applyOrder, moveCategory as placeCategory, moveChannel as placeChannel, orderOf, sidebarLayout, stepCategory, stepChannel, type Order, type Section } from '../lib/layout'
  import { sortableList, type Drop } from '../lib/drag'
  import { drafts, saveOrder } from '../lib/layoutDraft.svelte'
  import { categoryUi } from '../lib/categoryUi.svelte'
  import LiveBadge from './LiveBadge.svelte'
  import ParticipantMenu from './ParticipantMenu.svelte'
  import ChannelMenu from './ChannelMenu.svelte'
  import CategoryMenu from './CategoryMenu.svelte'
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

  type Kind = 'text' | 'quotes' | 'clips' | 'voice'
  const ICON: Record<string, IconName> = { text: 'hash', quotes: 'quote', clips: 'film', voice: 'speaker' }
  /** The kind picker in a category's new-channel field. */
  const KINDS: { kind: Kind; label: string }[] = [
    { kind: 'text', label: 'Text' }, { kind: 'quotes', label: 'Quotes' }, { kind: 'clips', label: 'Clips' }, { kind: 'voice', label: 'Voice' },
  ]
  const PLACEHOLDER: Record<Kind, string> = { text: 'new-channel', quotes: 'new-quotes', clips: 'new-clips', voice: 'Channel name' }
  /** An older server's kind sections: each + makes its own kind. */
  const LEGACY_ADD: Record<Kind, { icon: IconName; label: string }> = {
    text: { icon: 'plus', label: 'text' }, quotes: { icon: 'quote', label: 'quotes' }, clips: { icon: 'film', label: 'clips' }, voice: { icon: 'plus', label: 'voice' },
  }

  const self = $derived(selfVoice())
  // A reorder being saved shows at once, drawn over the server's layout.
  const layout = $derived.by(() => {
    const l = sidebarLayout(app.state, serverId)
    const draft = drafts.byServer[serverId]
    return draft && l.live ? applyOrder(l, draft) : l
  })
  // Who's in voice is only shown to people who've joined the server.
  const member = $derived(isMember(app.state, serverId))
  /** Members of a server with categories may reorder and manage them. */
  const arrange = $derived(member && layout.live)
  const order = (): Order => orderOf(layout)

  // Join tokens are fetched when the pointer rests on a voice channel, not while it sweeps past.
  const prefetch = hoverPrefetch((id) => voiceApi.prefetch(id))
  /** Hover and focus both start a prefetch; the channel you're already in needs no token. */
  const restOn = (id: string) => { if (member && id !== voiceChannelId) prefetch.enter(id) }
  const mutedForMe = (userId: string) => !!voice.localMutes[userId] && userId !== app.state.me?.id
  /** A voice row is a button: its label is all a screen reader reads, so it carries the state too. */
  const occupantLabel = (p: Occupant) =>
    [p.name, mutedForMe(p.userId) && 'muted for you', lowerFirst(serverVoiceTip(app.state, serverId, p)) ?? (p.deafened ? 'deafened' : p.muted && 'muted')].filter(Boolean).join(', ')
  const lowerFirst = (t: string | null) => (t ? t[0].toLowerCase() + t.slice(1) : null)
  $effect(() => () => prefetch.cancel())

  // the voice occupant under the pointer: its avatar animates
  let hoveredOccupant = $state<string | null>(null)

  // inline "new channel" field, under a section's heading: a category's has a kind picker, a legacy section's makes
  // its own kind
  let adding = $state<{ key: string; category: string | null } | null>(null)
  let addKind = $state<Kind>('text')
  let draft = $state('')
  let addError = $state('')

  function startAdd(key: string, category: Category | null, kind: Kind | null) {
    adding = { key, category: category?.id ?? null }
    addKind = kind ?? 'text'
    draft = ''
    addError = ''
  }
  async function submitAdd(e: KeyboardEvent) {
    if (e.key === 'Escape') adding = null
    if (e.key !== 'Enter' || !adding) return
    const { category } = adding
    const kind = addKind
    const name = tidyName(kind, draft)
    if (!name) return
    try {
      const c = category ? await api.createChannel(serverId, kind, name, category) : await api.createChannel(serverId, kind, name)
      adding = null
      if (kind !== 'voice') onSelect(c.id)
    } catch (err) {
      addError = errorText(err)
    }
  }
  /** Focus leaving the field and its kind picker cancels it. */
  function addFocusOut(e: FocusEvent) {
    if (!(e.currentTarget as HTMLElement).contains(e.relatedTarget as Node | null)) adding = null
  }
  /** The kind picker is a radio group: the arrow keys choose and move along it. */
  function kindKey(e: KeyboardEvent, i: number) {
    if (e.key === 'Escape') {
      e.preventDefault()
      adding = null
      return
    }
    const step = e.key === 'ArrowRight' || e.key === 'ArrowDown' ? 1 : e.key === 'ArrowLeft' || e.key === 'ArrowUp' ? -1 : 0
    if (!step) return
    e.preventDefault()
    const j = (i + step + KINDS.length) % KINDS.length
    addKind = KINDS[j].kind
    ;(e.currentTarget as HTMLElement).parentElement?.querySelectorAll<HTMLElement>('[role=radio]')[j]?.focus()
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
  /** Alt+Up / Alt+Down: -1 / 1, else 0. */
  const altStep = (e: KeyboardEvent) => (!e.altKey ? 0 : e.key === 'ArrowUp' ? -1 : e.key === 'ArrowDown' ? 1 : 0)
  function rowKey(e: KeyboardEvent, c: Channel) {
    if (e.key === 'ContextMenu' || (e.key === 'F10' && e.shiftKey)) {
      e.preventDefault()
      openChanMenu(c, e, e.currentTarget as HTMLElement)
    }
    const dir = altStep(e)
    if (dir && arrange) {
      e.preventDefault()
      moveChannel(c.id, dir)
    }
  }

  // Moving with the keyboard or the menus (dragging's other way, #54): the order is saved for everyone, shown at once
  // as a draft; focus stays on what moved (its element is new once it's in another section).
  async function refocus(selector: string) {
    await tick()
    listEl?.querySelector<HTMLElement>(selector)?.focus()
  }
  const headingSelector = (categoryId: string) => `.cat[data-category="${categoryId}"] > .label button.section`
  function moveChannel(id: string, dir: -1 | 1) {
    const o = order()
    const next = stepChannel(o, id, dir)
    if (!next) return
    // into a collapsed category: it opens, so the row doesn't vanish from under you
    const from = o.cats.find((c) => c.channels.includes(id))?.id
    const to = next.cats.find((c) => c.channels.includes(id))?.id
    if (to && to !== from) setCollapsed(`cat:${to}`, false)
    void saveOrder(serverId, next)
    void refocus(`.row[data-channel="${id}"]`)
  }
  function moveCategory(id: string, dir: -1 | 1) {
    const next = stepCategory(order(), id, dir)
    if (!next) return
    void saveOrder(serverId, next)
    void refocus(headingSelector(id))
  }

  // Dragging (#55): the drop is saved like a keyboard move, as a draft shown at once (the drag action clears its
  // transforms in the same frame this renders).
  function dropped(d: Drop) {
    const o = order()
    let next: Order
    if (d.kind === 'category') next = placeCategory(o, d.id, d.index)
    else {
      let { category, index } = d.to
      const key = `cat:${category}`
      if (category && isCollapsed(key)) {
        // a collapsed category shows only some of its rows: the drop is after the shown row above it; and it opens,
        // so the row doesn't vanish from under you
        const sec = layout.sections.find((x) => x.category?.id === category)
        const all = (sec?.channels ?? []).filter((c) => c.id !== d.id)
        const above = all.filter((c) => shown(c, key))[index - 1]
        index = above ? all.indexOf(above) + 1 : 0
        setCollapsed(key, false)
      }
      next = placeChannel(o, d.id, { category, index })
    }
    if (JSON.stringify(next) !== JSON.stringify(o)) void saveOrder(serverId, next)
  }
  const dragOpts = { enabled: () => arrange, onDrop: dropped, onHoverSection: (key: string) => setCollapsed(key, false) }

  // a category heading's menu (members, categories only): right-click, or Shift+F10 / the menu key on the heading
  let catMenu = $state<{ category: Category; x: number; y: number; opener: HTMLElement } | null>(null)
  function headingKey(e: KeyboardEvent, sec: Section) {
    if (!arrange || !sec.category) return
    if (e.key === 'ContextMenu' || (e.key === 'F10' && e.shiftKey)) {
      e.preventDefault()
      chanError = ''
      catMenu = { category: sec.category, ...menuPoint(e, e.currentTarget as HTMLElement), opener: e.currentTarget as HTMLElement }
    }
    const dir = altStep(e)
    if (dir) {
      e.preventDefault()
      moveCategory(sec.category.id, dir)
    }
  }
  function headingMenu(e: MouseEvent, sec: Section) {
    if (!arrange || !sec.category) return
    e.preventDefault()
    chanError = ''
    catMenu = { category: sec.category, ...menuPoint(e, e.currentTarget as HTMLElement), opener: e.currentTarget as HTMLElement }
  }

  // renaming a category happens in its heading, as a channel's does in its row
  let renamingCat = $state<string | null>(null)
  let catValue = $state('')
  let catError = $state('')
  let catSaving = false
  function startCatRename(c: Category) {
    renamingCat = c.id
    catValue = c.name
    catError = ''
  }
  async function endCatRename(id: string, refocusHeading: boolean) {
    if (renamingCat !== id) return
    renamingCat = null
    if (refocusHeading) await refocus(headingSelector(id))
  }
  /** Enter saves (and keeps the field if refused); leaving the field saves too, unless it was refused. */
  async function saveCatRename(c: Category, fromKey: boolean) {
    if (renamingCat !== c.id || catSaving) return
    if (!fromKey && catError) return void endCatRename(c.id, false)
    const name = catValue.trim()
    if (!name || name === c.name) return void endCatRename(c.id, fromKey)
    catSaving = true
    try {
      await api.renameCategory(c.id, name)
      await endCatRename(c.id, fromKey)
    } catch (err) {
      catError = errorText(err)
    } finally {
      catSaving = false
    }
  }
  function catRenameKey(e: KeyboardEvent, c: Category) {
    if (e.key === 'Enter') void saveCatRename(c, true)
    if (e.key === 'Escape') {
      e.preventDefault()
      void endCatRename(c.id, true)
    }
  }

  // "Create category" (the server menus): a heading-styled field at the end of the list
  let newCat = $state<{ serverId: string } | null>(null)
  let catDraft = $state('')
  let newCatError = $state('')
  $effect(() => {
    const asked = categoryUi.create
    if (!asked || asked.serverId !== serverId) return
    untrack(() => {
      newCat = { serverId }
      catDraft = ''
      newCatError = ''
      categoryUi.create = null
    })
  })
  async function newCatKey(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.preventDefault()
      newCat = null
    }
    if (e.key !== 'Enter') return
    const name = catDraft.trim()
    if (!name) return
    try {
      await api.createCategory(serverId, name)
      newCat = null
    } catch (err) {
      newCatError = errorText(err)
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
  /** Back to the row; from the keyboard, focus goes back to it too (the field it had is gone). */
  async function endRename(id: string, refocusRow: boolean) {
    if (renaming !== id) return
    renaming = null
    if (refocusRow) await refocus(`.row[data-channel="${id}"]`)
  }
  /** Enter saves (and keeps the field if refused); leaving the field saves too, unless it was refused. */
  async function saveRename(c: Channel, fromKey: boolean) {
    if (renaming !== c.id || renameSaving) return
    if (!fromKey && renameError) return void endRename(c.id, false)
    const name = tidyName(c.kind, renameValue)
    if (!name || name === c.name) return void endRename(c.id, fromKey)
    renameSaving = true
    try {
      await api.renameChannel(c.id, name)
      await endRename(c.id, fromKey)
    } catch (err) {
      renameError = errorText(err)
    } finally {
      renameSaving = false
    }
  }
  function renameKey(e: KeyboardEvent, c: Channel) {
    if (e.key === 'Enter') void saveRename(c, true)
    if (e.key === 'Escape') {
      e.preventDefault()
      void endRename(c.id, true)
    }
  }
  let deleting = $state<Channel | null>(null)
  let listEl = $state<HTMLElement>()
  // Deleted meanwhile (by anyone): its menu, rename, field or dialog has nothing left to act on.
  $effect(() => {
    const has = (id: string) => !!app.state.channels[id]
    if (chanMenu && !has(chanMenu.channel.id)) chanMenu = null
    if (deleting && !has(deleting.id)) deleting = null
    if (renaming && !has(renaming)) renaming = null
    const cats = new Set((app.state.layouts[serverId]?.categories ?? []).map((c) => c.id))
    if (catMenu && !cats.has(catMenu.category.id)) catMenu = null
    if (renamingCat && !cats.has(renamingCat)) renamingCat = null
    if (adding?.category && !cats.has(adding.category)) adding = null
  })

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
  /** In a collapsed section only these rows stay: the channel you're on, unread unmuted ones, and the voice channel
   *  you're in. The top area has no heading, so it never collapses. */
  const shown = (c: Channel, key: string | null) =>
    !key || !isCollapsed(key) || c.id === activeChannelId ||
    (c.kind === 'voice' ? c.id === voiceChannelId : (app.state.reads[c.id]?.unread ?? 0) > 0 && !isMuted(app.state, c, clock.nowIso))
</script>

{#snippet heading(sec: Section, first: boolean)}
  {@const open = !isCollapsed(sec.key)}
  {#if sec.category && renamingCat === sec.category.id}
    {@const cat = sec.category}
    <div class="label renaming" class:first>
      <input class="input input-sm grow" use:focus bind:value={catValue} maxlength="64" aria-label="Rename category {cat.name}"
        onkeydown={(e) => catRenameKey(e, cat)} onblur={() => void saveCatRename(cat, false)} />
    </div>
    {#if catError}<small class="err" role="alert">{catError}</small>{/if}
  {:else}
    <div class="label" class:first>
      <button type="button" class="section" aria-expanded={open} aria-haspopup={arrange && sec.category ? 'menu' : undefined}
        onclick={() => toggleSection(sec.key)} oncontextmenu={(e) => headingMenu(e, sec)} onkeydown={(e) => headingKey(e, sec)}>
        <span class="caret" class:shut={!open}><Icon name="chevron" size={11} /></span>{sec.name}
      </button>
      {#if member && sec.kind}
        {@const k = sec.kind as Kind}
        <button class="add" aria-label="Create {LEGACY_ADD[k].label} channel" onclick={() => startAdd(sec.key, null, k)}><Icon name={LEGACY_ADD[k].icon} size={14} /></button>
      {:else if arrange && sec.category}
        {@const cat = sec.category}
        <button class="add" aria-label="Create channel in {sec.name}" onclick={() => startAdd(sec.key, cat, null)}><Icon name="plus" size={14} /></button>
      {/if}
    </div>
  {/if}
{/snippet}

{#snippet addField(sec: Section)}
  {#if sec.kind}
    <!-- an older server's section: one kind, as before categories -->
    <input class="input input-sm new" use:focus bind:value={draft} onkeydown={submitAdd} onblur={() => (adding = null)} placeholder={PLACEHOLDER[sec.kind as Kind]} maxlength="64" aria-label="New {sec.kind} channel name" />
  {:else}
    <div class="new new-row" onfocusout={addFocusOut}>
      <input class="input input-sm grow" use:focus bind:value={draft} onkeydown={submitAdd} placeholder={PLACEHOLDER[addKind]} maxlength="64" aria-label="New channel name" />
      <div class="kinds" role="radiogroup" aria-label="Channel kind">
        {#each KINDS as k, i (k.kind)}
          <!-- a press keeps focus in the name field (it would leave the field and cancel it) -->
          <button type="button" class="add" role="radio" aria-checked={addKind === k.kind} aria-label={k.label} data-tip={k.label} tabindex={addKind === k.kind ? 0 : -1}
            onmousedown={(e) => e.preventDefault()} onclick={() => (addKind = k.kind)} onkeydown={(e) => kindKey(e, i)}><Icon name={ICON[k.kind]} size={14} /></button>
        {/each}
      </div>
    </div>
  {/if}
  {#if addError}<small class="err">{addError}</small>{/if}
{/snippet}

{#snippet renameField(c: Channel, icon: IconName)}
  <div class="row renaming">
    <span class="ico"><Icon name={icon} size={17} /></span>
    <input class="input input-sm grow" use:focus bind:value={renameValue} maxlength="64" aria-label="Rename {c.kind === 'voice' ? c.name : `#${c.name}`}"
      onkeydown={(e) => renameKey(e, c)} onblur={() => void saveRename(c, false)} />
  </div>
  {#if renameError}<small class="err" role="alert">{renameError}</small>{/if}
{/snippet}

{#snippet channelRow(c: Channel, icon: IconName)}
  {@const read = app.state.reads[c.id]}
  {@const muted = isMuted(app.state, c, clock.nowIso)}
  {#if renaming === c.id}
    {@render renameField(c, icon)}
  {:else}
  <button class="row" class:active={c.id === activeChannelId} class:unread={(read?.unread ?? 0) > 0 && !muted} class:muted aria-haspopup="menu" data-channel={c.id}
    onclick={() => onSelect(c.id)}
    oncontextmenu={(e) => { e.preventDefault(); openChanMenu(c, e, e.currentTarget) }} onkeydown={(e) => rowKey(e, c)}>
    <span class="ico"><Icon name={icon} size={17} /></span><span class="grow">{c.name}</span>
    {#if muted}<span class="mic"><Icon name="bellOff" size={14} /></span>{/if}
    {#if read?.mentions}<span class="pill">{read.mentions}</span>{/if}
  </button>
  {/if}
{/snippet}

{#snippet voiceRow(c: Channel)}
  {@const people = member ? callRoster(app.state, serverId, c.id, self) : []}
  {#if renaming === c.id}
    {@render renameField(c, 'speaker')}
  {:else}
  <button class="row" class:active={c.id === activeChannelId} class:live={c.id === voiceChannelId} aria-haspopup={member ? 'menu' : undefined} data-channel={c.id}
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
      <span class="grow"><Name userId={p.userId} {serverId} interactive={false} surface="side" /></span>
      {#if p.pending}
        <span class="joining">Joining…</span><span class="spinner" role="progressbar" aria-label="Joining"></span>
      {:else}
        <LiveBadge channelId={c.id} userId={p.userId} />
        <!-- the row's label says these (a button's content isn't read): the icons are decoration -->
        {#if mutedForMe(p.userId)}<span class="flag for-me" data-tip="Muted for you (they don't know)"><Icon name="speakerOff" size={14} /></span>{/if}
        {#if serverVoiceTip(app.state, serverId, p)}<span class="flag server" data-tip={serverVoiceTip(app.state, serverId, p)}><Icon name={p.serverDeafenedBy ? 'headphonesOff' : 'micOff'} size={12} /></span>
        {:else if p.deafened}<span class="flag" data-tip="Deafened"><Icon name="headphonesOff" size={14} /></span>
        {:else if p.muted}<span class="flag" data-tip="Muted"><Icon name="micOff" size={14} /></span>{/if}
      {/if}
    </div>
  {/each}
{/snippet}

{#snippet chan(c: Channel)}
  {#if c.kind === 'voice'}{@render voiceRow(c)}{:else}{@render channelRow(c, ICON[c.kind] ?? 'hash')}{/if}
{/snippet}

<div class="list" bind:this={listEl} use:sortableList={dragOpts}>
  <div class="cat top">
    {#each layout.top as c (c.id)}{@render chan(c)}{/each}
  </div>
  {#each layout.sections as sec, i (sec.key)}
    <div class="cat" data-category={sec.category?.id}>
      {@render heading(sec, i === 0 && layout.top.length === 0)}
      {#if adding?.key === sec.key}{@render addField(sec)}{/if}
      {#each sec.channels as c (c.id)}{#if shown(c, sec.key)}{@render chan(c)}{/if}{/each}
    </div>
  {/each}
  {#if newCat?.serverId === serverId}
    <div class="label new-cat" class:first={layout.top.length === 0 && layout.sections.length === 0}>
      <input class="input input-sm grow" use:focus bind:value={catDraft} maxlength="64" placeholder="New category" aria-label="New category name"
        onkeydown={newCatKey} onblur={() => (newCat = null)} />
    </div>
    {#if newCatError}<small class="err" role="alert">{newCatError}</small>{/if}
  {/if}
  {#if chanError}<small class="err" role="alert">{chanError}</small>{/if}
</div>
{#if chanMenu}
  {@const c = chanMenu.channel}
  {#key chanMenu}<ChannelMenu channel={c} {member} x={chanMenu.x} y={chanMenu.y} opener={chanMenu.opener} onClose={() => (chanMenu = null)}
    onRename={() => startRename(c)} onDelete={() => (deleting = c)} onJoin={() => onJoinVoice(c.id)} onError={(m) => (chanError = m)}
    onMoveUp={arrange ? () => moveChannel(c.id, -1) : undefined} onMoveDown={arrange ? () => moveChannel(c.id, 1) : undefined}
    canMoveUp={arrange && !!stepChannel(order(), c.id, -1)} canMoveDown={arrange && !!stepChannel(order(), c.id, 1)} />{/key}
{/if}
{#if catMenu}
  {@const cat = catMenu.category}
  {#key catMenu}<CategoryMenu category={cat} x={catMenu.x} y={catMenu.y} opener={catMenu.opener} onClose={() => (catMenu = null)}
    canMoveUp={!!stepCategory(order(), cat.id, -1)} canMoveDown={!!stepCategory(order(), cat.id, 1)}
    onCreateChannel={() => startAdd(`cat:${cat.id}`, cat, null)} onRename={() => startCatRename(cat)}
    onMoveUp={() => moveCategory(cat.id, -1)} onMoveDown={() => moveCategory(cat.id, 1)} onError={(m) => (chanError = m)} />{/key}
{/if}
{#if deleting}
  <DeleteChannelDialog channel={deleting} onClose={() => (deleting = null)} />
{/if}
{#if menu}
  {#key menu}<ParticipantMenu channelId={menu.channelId} {serverId} userId={menu.userId} x={menu.x} y={menu.y} opener={menu.opener} {onMessage} {onVoiceSettings} onClose={() => (menu = null)} />{/key}
{/if}

<style>
  .list { position: relative; flex: 1; padding: 8px; display: flex; flex-direction: column; gap: 2px; overflow-y: auto; }
  /* dragging (#55, lib/drag.ts): transforms only. What makes room slides; the lifted item follows the pointer (no
     transition) until it's dropped or sent back (returning); the frame a drop renders in has no transitions at all. */
  .list:global(.dragging) { user-select: none; touch-action: none; cursor: grabbing; }
  .list:global(.dragging) :global(*) { cursor: grabbing; }
  .list :global(.shift) { transition: transform var(--dur) var(--ease); }
  .list :global(.lifted) { position: relative; z-index: 2; background: var(--bg-2); box-shadow: var(--shadow-1); transition: none; }
  .list :global(.cat.lifted) { border-radius: var(--radius-sm); }
  .list :global(.lifted.returning) { transition: transform var(--dur) var(--ease); }
  .list:global(.settling) :global(*) { transition: none !important; }
  .list :global(.drop-line) { position: absolute; top: 0; left: 8px; right: 8px; height: 2px; border-radius: var(--radius-xs); background: var(--accent); pointer-events: none; z-index: 1; transition: transform var(--dur-fast) var(--ease); }
  /* a section: its heading, then its rows (the top area has no heading, and nothing to show when empty) */
  .cat { display: flex; flex-direction: column; gap: 2px; }
  .cat.top:empty { display: none; }
  .label { padding: 16px 10px 6px; font-size: 11px; font-weight: 600; letter-spacing: .06em; color: var(--text-3); text-transform: uppercase; }
  .label { display: flex; align-items: center; justify-content: space-between; }
  .label.first { padding-top: 12px; }
  .label.renaming, .label.new-cat { padding-left: 4px; padding-right: 4px; }
  /* the heading's name hides and shows the section; shown upper-case, stored as typed */
  .section { display: flex; align-items: center; gap: 4px; padding: 0; border: 0; background: none; color: inherit; font: inherit; letter-spacing: inherit; text-transform: inherit; cursor: pointer; }
  .section:hover { color: var(--text-2); }
  .caret { display: flex; transition: transform var(--dur) var(--ease); }
  .caret.shut { transform: rotate(-90deg); }
  .add { width: 20px; height: 20px; border: 0; border-radius: var(--radius-xs); background: transparent; color: var(--text-3); display: grid; place-items: center; padding: 0; flex-shrink: 0; }
  .add:hover { color: var(--text); background: var(--bg-2); }
  .add[aria-checked='true'] { background: var(--bg-3); color: var(--text); }
  .new { margin: 2px 4px; }
  .new-row { display: flex; align-items: center; gap: 4px; }
  .kinds { display: flex; gap: 2px; }
  .row.renaming { padding-right: 4px; }
  .row.renaming:hover { background: transparent; }
  .err { padding: 0 10px; color: var(--danger-text); font-size: 12px; }
  .row { height: 34px; padding: 0 10px; border: 0; border-radius: var(--radius-sm); background: transparent; color: var(--text-row); display: flex; align-items: center; gap: 8px; text-align: left; font-size: 14px; }
  .row:hover { background: var(--bg-2); color: var(--text); }
  /* background and colour only: a bolder weight would widen the name */
  .row.active { background: var(--bg-3); color: var(--text); }
  .row.live .ico { color: var(--ok-text); }
  .ico { color: var(--text-3); display: flex; }
  .grow { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .count { font-size: 11px; color: var(--text-3); }
  .occupant { cursor: default; border-radius: var(--radius-sm); height: 30px; padding: 0 8px 0 38px; display: flex; align-items: center; gap: 9px; font-size: 13px; color: var(--text-2); }
  /* the ring snaps with speaking, as the voice tile's border does (no box-shadow transition: a repaint per frame) */
  .ring { display: flex; flex-shrink: 0; border-radius: 50%; }
  .ring.speaking { box-shadow: 0 0 0 2px var(--ring), 0 0 0 4px var(--ok); }
  .occupant:hover { background: var(--bg-2); }
  .occupant.pending { opacity: .6; }
  .joining { font-size: 11px; color: var(--text-2); }
  .spinner { width: 12px; height: 12px; border-radius: 50%; border: 2px solid var(--text-3); border-top-color: transparent; animation: spin .8s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
  @media (prefers-reduced-motion: reduce) { .spinner { animation-duration: 2.4s; } }
  .flag { color: var(--danger-text); display: flex; }
  .flag.for-me { color: var(--warn-text); }
  /* the server's mute (#140), not theirs: a filled badge */
  .flag.server { background: var(--danger); color: var(--on-danger); border-radius: 5px; padding: 2px; }
  .row.unread { color: var(--text); font-weight: 600; }
  .row.muted { opacity: .5; }
  .mic { color: var(--text-3); display: grid; }
  .pill { min-width: 16px; height: 16px; border-radius: var(--radius-sm); background: var(--danger); color: var(--on-danger); font-size: 10px; font-weight: 700; display: inline-block; box-sizing: border-box; text-align: center; line-height: normal; text-box: trim-both cap alphabetic; padding: calc((16px - 1cap) / 2) 4px; }
</style>
