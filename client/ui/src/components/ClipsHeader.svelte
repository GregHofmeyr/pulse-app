<script lang="ts">
  // A clips channel's header: its name, the list/wall toggle, the pins (#257), "Share a clip", search and, rightmost,
  // the toggle of the filters panel (#663: games and people, as Builds). With the panel closed, a filter that is on
  // still shows here, so a filtered list never looks like everything.
  import { tick } from 'svelte'
  import Icon from './Icon.svelte'
  import PinsPopover from './PinsPopover.svelte'
  import SearchBox from './SearchBox.svelte'
  import type { Message } from '../lib/protocol/Message'
  import { narrowerThan } from '../lib/actions'
  import type { ClipFilter } from '../lib/clips'
  import { displayName } from '../lib/selectors'
  import { app } from '../lib/store.svelte'
  import { api, errorText } from '../lib/tauri'
  import type { ClipView } from '../lib/protocol/ClipView'

  let { channelId, serverId, filter, onFilter, view, onView = () => {}, onShare, onOpenPinned = () => {}, panel = true, onPanel, panelId }: {
    /** A pinned clip's Open, from the pins (#257): a clips channel has no history to jump in, so its panel opens. */
    onOpenPinned?: (clip: Message) => void
    channelId: string
    serverId: string
    filter: ClipFilter
    /** "Clear filters" (shown with the panel closed). */
    onFilter: (f: ClipFilter) => void
    /** The view shown (the caller reads it from state; the toggle updates state itself). */
    view: ClipView
    /** Told after the toggle switched the view. */
    onView?: (v: ClipView) => void
    onShare: () => void
    /** Whether the filters panel is on screen. */
    panel?: boolean
    onPanel?: () => void
    /** The panel's id, for the toggle's `aria-controls`. */
    panelId?: string
  } = $props()

  const channel = $derived(app.state.channels[channelId])
  const games = $derived(app.state.games[serverId] ?? [])
  const filtering = $derived(
    [
      filter.game ? (games.find((g) => g.id === filter.game)?.name ?? 'Game') : null,
      filter.author ? displayName(app.state, serverId, filter.author) : null,
    ].filter((x): x is string => !!x).join(' · '),
  )

  // "Clear filters" goes with the summary it sits in, so the focus moves to the panel toggle.
  let panelBtn = $state<HTMLButtonElement>()
  async function clear() {
    onFilter({})
    await tick()
    panelBtn?.focus()
  }

  // A narrow header (small window, side panels open; #97): the search box folds to a magnifier.
  const NARROW_HEAD = 720
  let narrow = $state(false)

  let error = $state('')
  // the channel's pins (#257), the popover a chat's header opens
  let pinsOpen = $state(false)
  function openPinned(m: Message) {
    pinsOpen = false
    onOpenPinned(m)
  }

  // Optimistic: the view switches now, and goes back if the server refuses.
  function setView(v: ClipView) {
    if (v === view) return
    const was = view
    error = ''
    app.state = { ...app.state, clipViews: { ...app.state.clipViews, [channelId]: v } }
    onView(v)
    api.setClipView(channelId, v).catch((e) => {
      if (app.state.clipViews[channelId] === v) app.state = { ...app.state, clipViews: { ...app.state.clipViews, [channelId]: was } }
      error = errorText(e)
    })
  }
</script>

<div class="head" use:narrowerThan={{ px: NARROW_HEAD, set: (v) => (narrow = v) }}>
  <!-- the name and the filter summary: one line that truncates; a zero basis keeps it out of where the header's row
       breaks, so opening or closing the panel never changes the header's height -->
  <div class="name-group">
    <span class="ico"><Icon name="film" size={18} /></span>
    <h2>{channel?.name ?? ''}</h2>
    {#if !panel && filtering}
      <span class="filtering"><span class="what">{filtering}</span><button type="button" class="clear" aria-label="Clear filters" data-tip="Clear filters"
        onclick={clear}><Icon name="x" size={12} /></button></span>
    {/if}
  </div>
  {#if error}<span class="err" role="alert">{error}</span>{/if}
  <div class="toggle" role="group" aria-label="View">
    <button type="button" class="tbtn" aria-label="List view" data-tip="List" aria-pressed={view === 'list'} onclick={() => setView('list')}><Icon name="list" size={16} /></button>
    <button type="button" class="tbtn" aria-label="Wall view" data-tip="Wall" aria-pressed={view === 'wall'} onclick={() => setView('wall')}><Icon name="grid" size={16} /></button>
  </div>
  <button type="button" class="icon-btn hbtn" class:on={pinsOpen} aria-label="Pinned clips" data-tip="Pinned clips" data-menu-toggle aria-haspopup="dialog" aria-expanded={pinsOpen}
    onclick={() => (pinsOpen = !pinsOpen)}><Icon name="pin" size={17} /></button>
  <button type="button" class="share btn btn-primary" onclick={onShare}>Share a clip</button>
  <SearchBox {serverId} {channelId} compact={narrow} />
  {#if pinsOpen}
    <PinsPopover {channelId} {serverId} onJump={openPinned} onClose={() => (pinsOpen = false)} />
  {/if}
  {#if onPanel}
    <button type="button" class="icon-btn hbtn" class:on={panel} aria-label={panel ? 'Hide filters panel' : 'Show filters panel'} data-tip={panel ? 'Hide panel' : 'Show panel'}
      aria-expanded={panel} aria-controls={panel ? panelId : undefined} onclick={onPanel} bind:this={panelBtn}><Icon name="panelRight" size={17} /></button>
  {/if}
</div>

<style>
  .head { position: relative; min-height: 52px; flex-shrink: 0; padding: 8px 18px; box-sizing: border-box; display: flex; align-items: center; flex-wrap: wrap; gap: 10px; border-bottom: 1px solid var(--bg-3); color: var(--text-3); }
  /* the pins button, as a chat header's (TextChannel .hbtn) */
  .hbtn.on { background: var(--bg-3); color: var(--text); }
  .name-group { flex: 1 1 0; min-width: 0; display: flex; flex-wrap: nowrap; align-items: center; gap: 10px; }
  .ico { display: flex; flex-shrink: 0; }
  h2 { flex: 0 1 auto; margin: 0; min-width: 0; max-width: 240px; font-size: 15px; font-weight: 600; color: var(--text); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  /* the filter that is on, with the panel closed: one line that ellipsizes, never pushes the header around */
  .filtering { flex: 0 2 auto; display: flex; align-items: center; gap: 2px; min-width: 0; max-width: 260px; padding: 2px 2px 2px 8px; border-radius: var(--radius-sm); background: var(--bg-3); color: var(--text-2); font-size: 13px; }
  .what { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .clear { flex-shrink: 0; display: grid; place-items: center; width: 20px; height: 20px; padding: 0; border: 0; border-radius: var(--radius-sm); background: transparent; color: var(--text-3); }
  .clear:hover { background: var(--bg-4); color: var(--text); }
  .err { color: var(--danger-text); font-size: 12px; }
  .toggle { display: flex; padding: 2px; border-radius: var(--radius-md); background: var(--bg-2); }
  .tbtn { width: 30px; height: 28px; border: 0; border-radius: var(--radius-sm); background: transparent; color: var(--text-3); display: grid; place-items: center; }
  .tbtn:hover { color: var(--text); }
  .tbtn[aria-pressed='true'] { background: var(--bg-4); color: var(--text); }
  /* the app's accent button (.btn .btn-primary), at the header's 32 px */
  .share { padding: 0 14px; }
</style>
