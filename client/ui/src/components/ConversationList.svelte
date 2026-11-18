<script lang="ts">
  import Icon from './Icon.svelte'
  import Avatar from './Avatar.svelte'
  import GroupIcon from './GroupIcon.svelte'
  import Name from './Name.svelte'
  import { app, clock } from '../lib/store.svelte'
  import { presenceOf } from '../lib/presence'
  import CustomStatusLine from './CustomStatusLine.svelte'
  import { customStatusOf } from '../lib/customStatus'
  import { displayName, liveIn } from '../lib/selectors'
  import ConversationMenu from './ConversationMenu.svelte'
  import LiveBadge from './LiveBadge.svelte'
  import { conversations, NOTES_NAME, previewParts, previewText } from '../lib/conversations'
  import { mediaUrl } from '../lib/emoji'
  import { voice } from '../lib/voice.svelte'
  import { openDm } from '../lib/store.svelte'
  import { errorText } from '../lib/tauri'
  import DmCategoryMenu from './DmCategoryMenu.svelte'
  import { DM_CATEGORY_CHARS, categoryNameOk, categoryNameReserved, dmSections } from '../lib/dmLayout'
  import { placeConversation, renameCategory, setCategoryCollapsed } from '../lib/dmLayout.svelte'
  import { placementOf } from '../lib/dmLayout'
  import { convDrag, dropPlacement, type DropTarget } from '../lib/dmDrag'
  import { isMenuKey, menuPoint } from '../lib/participantMenu'
  import type { DmCategory } from '../lib/protocol/DmCategory'

  let { activeId, onOpen, onNew, onLeft }: {
    activeId: string | null
    onOpen: (id: string) => void
    onNew: () => void
    /** A conversation was closed or left from its right-click menu. */
    onLeft: (id: string) => void
  } = $props()

  // Right-click a conversation: mute, close, leave.
  let ctx = $state<null | { id: string; x: number; y: number }>(null)

  const list = $derived(conversations(app.state, clock.nowIso))
  const me = $derived(app.state.me?.id)
  const others = (members: string[]) => members.filter((u) => u !== me)
  const nameOf = (id: string) => (app.state.people[id] ? displayName(app.state, null, id) : '?')
  // the row under the pointer: its avatars animate
  let hovered = $state<string | null>(null)
  const over = (id: string) => () => hovered === id
  /** Who's live in a conversation's call (#286), oldest stream first. */
  const streamers = (id: string) => Object.values(liveIn(app.state, id)).sort((a, b) => a.started_at.localeCompare(b.started_at)).map((x) => x.user_id)
  const liveTip = (ids: string[]) => `${ids.map(nameOf).join(', ')} ${ids.length === 1 ? 'is' : 'are'} live`

  // Your pins and categories (#305): Pinned, then each of your categories (folded open or shut, from its heading;
  // right-click it to rename or delete it), then the rest. Moving a conversation is in its right-click menu.
  const sections = $derived(dmSections(list, app.state.dmLayout))
  let layoutError = $state('')
  const report = (f: () => Promise<unknown>) => {
    layoutError = ''
    return f().catch((e) => (layoutError = errorText(e)))
  }
  const toggle = (id: string, collapsed: boolean) => report(() => setCategoryCollapsed(id, collapsed))
  // Drag a conversation onto a category, Pinned or the main list (#419; lib/dmDrag.ts). While one is lifted, an empty
  // Pinned and main list show as places to drop too.
  let lifted = $state(false)
  const canDrag = $derived(sections.categories.length > 0 || sections.pinned.length > 0)
  function dropped(id: string, target: DropTarget) {
    const p = placementOf(app.state.dmLayout, id)
    const next = dropPlacement({ pinned: p?.pinned ?? false, category_id: p?.category_id ?? null }, target)
    if (next) void report(() => placeConversation(id, next))
  }
  let catMenu = $state<null | { category: DmCategory; x: number; y: number; opener: HTMLElement }>(null)
  function openCatMenu(e: MouseEvent | KeyboardEvent, category: DmCategory) {
    e.preventDefault()
    const opener = e.currentTarget as HTMLElement
    catMenu = { category, ...menuPoint(e, opener), opener }
  }
  function catKey(e: KeyboardEvent, category: DmCategory) {
    if (isMenuKey(e)) openCatMenu(e, category)
  }
  let renaming = $state<string | null>(null)
  let renameValue = $state('')
  const focus = (el: HTMLInputElement) => el.focus()
  function startRename(category: DmCategory) {
    renaming = category.id
    renameValue = category.name
  }
  async function saveRename(id: string) {
    if (renaming !== id) return
    renaming = null
    const was = app.state.dmLayout.categories.find((c) => c.id === id)?.name
    if (categoryNameReserved(renameValue)) return void (layoutError = `"${renameValue.trim()}" is a menu entry: pick another name`)
    if (!categoryNameOk(renameValue) || renameValue.trim() === was) return
    await report(() => renameCategory(id, renameValue))
  }
  function renameKey(e: KeyboardEvent, id: string) {
    if (e.key === 'Enter' && !e.isComposing) {
      e.preventDefault()
      void saveRename(id)
    } else if (e.key === 'Escape') {
      e.preventDefault()
      renaming = null
    }
  }

  // Notes to self (#308): always on top. Before its first open (or after you closed it) the row stands in for it, and
  // opening it asks the server for your DM with only you, which makes it the first time.
  const hasNotes = $derived(list.some((c) => c.notes))
  let notesError = $state('')
  async function openNotes() {
    if (!me) return
    notesError = ''
    try {
      onOpen(await openDm(me))
    } catch (e) {
      notesError = errorText(e)
    }
  }
</script>

{#snippet notesPicture()}
  <span class="notes-pic" aria-hidden="true"><Icon name="pencil" size={16} /></span>
{/snippet}

{#snippet picture(c: (typeof list)[number], people: string[])}
  {#if c.notes}
    {@render notesPicture()}
  {:else if c.channel.kind === 'dm' && people[0]}
    <Avatar id={people[0]} name={nameOf(people[0])} size={32} presence={presenceOf(app.state, people[0], clock.nowIso)} hovered={over(c.channel.id)} />
  {:else if c.channel.icon_hash}
    <GroupIcon hash={c.channel.icon_hash} still={c.channel.icon_still} size={32} hovered={over(c.channel.id)} />
  {:else}
    <span class="stack">
      {#each people.slice(0, 2) as u, i (u)}
        <span class="s{i}"><Avatar id={u} name={nameOf(u)} size={21} hovered={over(c.channel.id)} /></span>
      {/each}
    </span>
  {/if}
{/snippet}

{#snippet row(c: (typeof list)[number])}
  {@const people = others(c.members)}
  {@const badge = c.muted ? c.mentions : c.unread}
  {@const live = streamers(c.channel.id)}
  {@const custom = c.channel.kind === 'dm' && people[0] && !(c.unread > 0 && !c.muted) ? customStatusOf(app.state, people[0], clock.nowIso) : null}
  <button class="row" data-conv={c.notes ? undefined : c.channel.id} class:active={c.channel.id === activeId} class:unread={c.unread > 0 && !c.muted} class:muted={c.muted}
    onclick={() => onOpen(c.channel.id)}
    onpointerenter={() => (hovered = c.channel.id)} onpointerleave={() => { if (hovered === c.channel.id) hovered = null }}
    oncontextmenu={(e) => { e.preventDefault(); if (c.notes) return; ctx ={ id: c.channel.id, x: e.clientX, y: Math.min(e.clientY, innerHeight - 250) } }}>
    {#if c.channel.id === voice.channelId}
      <!-- the call you're in (#220): the same ring and badge as its tab -->
      <span class="call-here round">{@render picture(c, people)}<span class="call-badge" aria-hidden="true"><Icon name="speaker" size={10} /></span></span>
    {:else}
      {@render picture(c, people)}
    {/if}
    <span class="meta">
      <!-- the row is a button: the DM's name is styled but not a button of its own -->
      <span class="name">{#if c.channel.kind === 'dm' && people[0]}<Name userId={people[0]} serverId={null} text={c.name} interactive={false} surface="side" />{:else}<span class="name-clip">{c.name}</span>{/if}</span>
      {#if custom}
        <!-- a 1:1 DM's other person's custom status (#152) takes the preview's line, unless there's something new -->
        <span class="prev"><CustomStatusLine status={custom} /></span>
      {:else if c.latest}
        {@const p = previewText(app.state, c.latest)}
        <span class="prev" class:sys={p.system}>{#each previewParts(app.state, p.text) as part, i (i)}{#if 'emoji' in part}<img class="emoji" src={mediaUrl(part.emoji.hash)} alt=":{part.emoji.name}:" draggable="false" />{:else}{part.text}{/if}{/each}</span>
      {/if}
    </span>
    <!-- a stream in its call (#286): the sidebar's LIVE badge, as a still mark (the row is the button) -->
    {#if live.length}<LiveBadge channelId={c.channel.id} userId={live[0]} marker={liveTip(live)} />{/if}
    {#if c.channel.id === voice.channelId}<span class="in-call mine" role="img" aria-label="You're in this call" data-tip="You're in this call"><Icon name="speaker" size={14} /></span>
    {:else if app.state.calls[c.channel.id]}<span class="in-call" role="img" aria-label="In call" data-tip="In call"><Icon name="phone" size={14} /></span>{/if}
    {#if c.muted}<span class="mute"><Icon name="bellOff" size={14} /></span>{/if}
    {#if badge > 0}<span class="pill">{badge > 99 ? '99+' : badge}</span>{/if}
  </button>
{/snippet}

<button class="btn btn-lg btn-quiet new" onclick={onNew}><Icon name="plus" size={16} /> New message</button>

<div class="list" use:convDrag={{ onDrop: dropped, onLift: (on) => (lifted = on), disabled: !canDrag }}>
  {#if !hasNotes}
    <button class="row" onclick={openNotes}>
      {@render notesPicture()}
      <span class="meta"><span class="name"><span class="name-clip">{NOTES_NAME}</span></span><span class="prev">Just for you</span></span>
    </button>
    {#if notesError}<p class="err" role="alert">{notesError}</p>{/if}
  {/if}
  {#each sections.notes as c (c.channel.id)}{@render row(c)}{/each}
  {#if sections.pinned.length || lifted}
    <div class="drop" data-drop="pinned">
      <div class="label"><span class="section static"><Icon name="pin" size={11} />Pinned</span></div>
      {#each sections.pinned as c (c.channel.id)}{@render row(c)}{/each}
    </div>
  {/if}
  {#each sections.categories as sec (sec.category.id)}
    {@const cat = sec.category}
    <div class="drop" data-drop="category:{cat.id}">
    {#if renaming === cat.id}
      <div class="label renaming">
        <input class="input input-sm grow" use:focus bind:value={renameValue} maxlength={DM_CATEGORY_CHARS} aria-label="Rename category {cat.name}"
          onkeydown={(e) => renameKey(e, cat.id)} onblur={() => void saveRename(cat.id)} />
      </div>
    {:else}
      {@const total = sec.items.reduce((n, c) => n + (c.muted ? c.mentions : c.unread), 0)}
      <div class="label">
        <button type="button" class="section" aria-expanded={!cat.collapsed} aria-haspopup="menu"
          onclick={() => void toggle(cat.id, !cat.collapsed)} oncontextmenu={(e) => openCatMenu(e, cat)} onkeydown={(e) => catKey(e, cat)}>
          <span class="caret" class:shut={cat.collapsed}><Icon name="chevron" size={11} /></span>{cat.name}
        </button>
        {#if cat.collapsed && total > 0}<span class="pill">{total > 99 ? '99+' : total}</span>{/if}
      </div>
    {/if}
    {#if !cat.collapsed}
      {#each sec.items as c (c.channel.id)}{@render row(c)}{/each}
      {#if !sec.items.length}<p class="none">Drag a conversation here, or right-click it to move it.</p>{/if}
    {:else}
      <!-- folded: only the one you're in stays in sight -->
      {#each sec.items.filter((c) => c.channel.id === activeId) as c (c.channel.id)}{@render row(c)}{/each}
    {/if}
    </div>
  {/each}
  {#if layoutError}<p class="err" role="alert">{layoutError}</p>{/if}
  <div class="drop" data-drop="rest">
    {#if (sections.pinned.length || sections.categories.length) && (sections.rest.length || lifted)}
      <div class="label"><span class="section static">Conversations</span></div>
    {/if}
    {#each sections.rest as c (c.channel.id)}{@render row(c)}{/each}
  </div>
  {#if !list.length}
    <p class="empty">No conversations yet. Say hi to someone from Your people.</p>
  {/if}
</div>

{#if ctx}
  {@const c = list.find((x) => x.channel.id === ctx!.id)}
  {#if c}
    {#key ctx}
      <ConversationMenu channel={c.channel} muted={c.muted} show="all" at={ctx}
        onClose={() => (ctx = null)} onLeft={() => onLeft(c.channel.id)} />
    {/key}
  {/if}
{/if}

{#if catMenu}
  {@const cat = catMenu.category}
  {#key catMenu}<DmCategoryMenu category={cat} x={catMenu.x} y={catMenu.y} opener={catMenu.opener} onClose={() => (catMenu = null)}
    onRename={() => startRename(cat)} onError={(m) => (layoutError = m)} />{/key}
{/if}

<style>
  .new { margin: 4px 8px 8px; font-size: 13px; }
  .list { flex: 1; min-height: 0; overflow-y: auto; padding: 0 6px; display: flex; flex-direction: column; gap: 1px; }
  .row { display: flex; align-items: center; gap: 10px; padding: 7px 8px; border: 0; border-radius: var(--radius-md); background: transparent; color: var(--text-row); text-align: left; }
  .row:hover { background: var(--bg-2); }
  .row.active { background: var(--bg-3); color: var(--text); }
  .row.muted { opacity: .5; }
  .meta { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  .name { font-weight: 600; font-size: 14px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .row.unread .name { color: var(--text); }
  .prev { font-size: 12px; color: var(--text-3); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .prev.sys { font-style: italic; }
  /* a custom emoji in the last message: its still, sized to the line like emoji in messages */
  .prev .emoji { width: 1.25em; height: 1.25em; vertical-align: -0.3em; object-fit: contain; }
  .stack { position: relative; width: 32px; height: 32px; flex-shrink: 0; }
  .stack .s0 { position: absolute; left: 0; top: 0; }
  .stack .s1 { position: absolute; left: 11px; top: 11px; border-radius: 50%; box-shadow: 0 0 0 2px var(--ring); }
  .mute { color: var(--text-3); display: grid; }
  .in-call { color: var(--ok-text); display: grid; }
  /* yours (#220): the accent speaker of the voice channel you're in, not the green "call in progress" phone */
  .in-call.mine { color: var(--accent-text); }
  /* the ring's gap follows the row under it */
  .row { --call-gap: var(--bg-1); }
  .row:hover { --call-gap: var(--bg-2); }
  .row.active { --call-gap: var(--bg-3); }
  /* the unread pill of the channel list and server tabs (#173) */
  .pill { min-width: 16px; height: 16px; border-radius: var(--radius-sm); background: var(--danger); color: var(--on-danger); font-size: 10px; font-weight: 700; display: inline-block; box-sizing: border-box; text-align: center; line-height: normal; text-box: trim-both cap alphabetic; padding: calc((16px - 1cap) / 2) 4px; }
  /* the headings: Pinned, your categories (#305), and the rest; as the channel list's */
  .label { display: flex; align-items: center; justify-content: space-between; padding: 12px 8px 4px; font-size: 11px; font-weight: 600; letter-spacing: .06em; color: var(--text-3); text-transform: uppercase; }
  .label.renaming { padding-left: 2px; padding-right: 2px; }
  .label .grow { flex: 1; min-width: 0; }
  .section { display: flex; align-items: center; gap: 4px; padding: 0; border: 0; background: none; color: inherit; font: inherit; letter-spacing: inherit; text-transform: inherit; cursor: pointer; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .section:hover:not(.static) { color: var(--text-2); }
  .section.static { cursor: default; }
  .caret { display: flex; transition: transform var(--dur) var(--ease); }
  .caret.shut { transform: rotate(-90deg); }
  .none { margin: 0; padding: 4px 10px 6px; font-size: 12px; color: var(--text-3); }
  .empty { padding: 12px 10px; font-size: 13px; color: var(--text-3); }
  /* your notes (#308): a round tile the size of a DM's avatar */
  .notes-pic { width: 32px; height: 32px; flex-shrink: 0; border-radius: 50%; display: grid; place-items: center; background: var(--bg-3); color: var(--text-2); }
  .err { padding: 0 10px 6px; font-size: 12px; color: var(--danger-text); }
  /* drag onto a section (#419): sections lay out as part of the list; the lifted row follows the pointer above the
     rest and takes no pointer (so what's under it is found); the section under it lights its heading */
  .drop { display: contents; }
  .row:global(.lifted) { position: relative; z-index: 5; pointer-events: none; opacity: .85; background: var(--bg-3); box-shadow: var(--shadow-2); }
  .list:global(.dragging) { cursor: grabbing; }
  .drop:global(.drop-here) > .label { color: var(--accent-text); background: var(--bg-2); border-radius: var(--radius-sm); }
</style>
