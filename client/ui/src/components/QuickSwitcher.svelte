<script lang="ts">
  // The quick switcher (#309): one floating search bar, its results unfolding underneath on the same surface (no
  // dialog header). Jump to a server, channel, DM or group; `>` for quick actions. The rules are in lib/quickSwitch.ts
  // and lib/quickActions.ts; the shell opens it (Ctrl+K, or `/` outside a text field) and does the going.
  import { tick } from 'svelte'
  import Icon, { type IconName } from './Icon.svelte'
  import Avatar from './Avatar.svelte'
  import GroupIcon from './GroupIcon.svelte'
  import { modal } from '../lib/actions'
  import { app } from '../lib/store.svelte'
  import { avatarColor, initial } from '../lib/avatar'
  import { allActions, type ActionContext } from '../lib/quickActions'
  import { buildResults, placesFrom, type Place, type Row } from '../lib/quickSwitch'

  let { context, recents, here, onGo, onClose }: {
    /** What the quick actions can see and do, read when the list builds (so Mute turns to Unmute). */
    context: () => ActionContext
    /** Recently visited place keys, newest first. */
    recents: readonly string[]
    /** Where you are now (left out of the recents). */
    here: string | null
    /** Go to `place`; `join` also joins a voice channel (its row's Join). */
    onGo: (place: Place, join: boolean) => void
    onClose: () => void
  } = $props()

  let raw = $state('')
  let active = $state(0)
  let input = $state<HTMLInputElement>()

  const places = $derived(placesFrom(app.state))
  const ctx = $derived(context())
  const actions = $derived(allActions(ctx))
  const sections = $derived(buildResults({ places, actions, raw, recents, here }))
  // one list for the keys: each section's rows follow on from the one before
  const flat = $derived(sections.flatMap((s) => s.rows))
  const starts = $derived(sections.reduce<number[]>((acc, _s, i) => [...acc, i === 0 ? 0 : acc[i - 1] + sections[i - 1].rows.length], []))
  const typed = $derived(raw.trim().length > 0)
  const optId = (i: number) => `qs-opt-${i}`
  const count = $derived(flat.length === 0 ? (typed ? 'No results' : '') : `${flat.length} result${flat.length === 1 ? '' : 's'}`)

  // a new query starts at the top
  $effect(() => {
    void raw
    active = 0
  })
  $effect(() => {
    if (active >= flat.length) active = Math.max(0, flat.length - 1)
  })

  async function move(by: number) {
    const n = flat.length
    if (!n) return
    active = (active + by + n) % n
    await tick()
    document.getElementById(optId(active))?.scrollIntoView?.({ block: 'nearest' })
  }

  async function choose(i: number, join = false) {
    const row = flat[i]
    if (!row) return
    // held now: closing tears this component down, props and all
    const go = onGo
    const c = ctx
    onClose()
    // after the switcher has gone and handed the focus back: what opens next (Settings, a channel) takes it from there
    await tick()
    if (row.type === 'place') go(row.place, join && row.place.kind === 'voice')
    else row.action.run(c)
  }

  function keys(e: KeyboardEvent) {
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp' || e.key === 'Tab') {
      e.preventDefault()
      // the list moves, not the focus: the dialog's own Tab wrap mustn't take it out of the field
      if (e.key === 'Tab') e.stopPropagation()
      void move(e.key === 'ArrowUp' || (e.key === 'Tab' && e.shiftKey) ? -1 : 1)
    } else if (e.key === 'Enter' && !e.isComposing) {
      e.preventDefault()
      void choose(active, e.shiftKey)
    }
  }

  const KIND_ICON: Partial<Record<Place['kind'], IconName>> = { text: 'hash', voice: 'speaker', quotes: 'quote', clips: 'film' }
  const otherIn = (id: string) => (app.state.dmMembers[id] ?? []).find((u) => u !== app.state.me?.id) ?? null
  const label = (r: Row) => (r.type === 'place' ? r.place.name : r.action.label)
</script>

{#snippet lead(r: Row)}
  {#if r.type === 'action'}
    <span class="lead"><Icon name={r.action.icon} size={18} /></span>
  {:else if r.place.kind === 'server'}
    <span class="lead chip" style:background={avatarColor(r.place.id)} aria-hidden="true">{initial(r.place.name)}</span>
  {:else if r.place.kind === 'dm'}
    {@const other = otherIn(r.place.id)}
    <span class="lead">{#if other}<Avatar id={other} name={r.place.name} size={20} />{:else}<Icon name="message" size={18} />{/if}</span>
  {:else if r.place.kind === 'group'}
    {@const ch = app.state.channels[r.place.id]}
    <span class="lead">{#if ch?.icon_hash}<GroupIcon hash={ch.icon_hash} still={ch.icon_still} size={20} />{:else}<Icon name="users" size={18} />{/if}</span>
  {:else}
    <span class="lead"><Icon name={KIND_ICON[r.place.kind] ?? 'hash'} size={18} /></span>
  {/if}
{/snippet}

<div class="overlay modal-overlay fade-in">
  <div class="backdrop modal-backdrop" role="presentation" data-backdrop onclick={onClose}></div>
  <div class="launcher pop-in" role="dialog" aria-modal="true" aria-label="Quick switcher" tabindex="-1" use:modal={onClose}>
    <div class="bar">
      <Icon name="search" size={18} />
      <!-- svelte-ignore a11y_autofocus -->
      <input bind:this={input} bind:value={raw} class="field" type="text" role="combobox" autofocus autocomplete="off" spellcheck="false"
        aria-label="Jump to" aria-autocomplete="list" aria-expanded={flat.length > 0} aria-controls="qs-list"
        aria-activedescendant={flat.length ? optId(active) : undefined}
        placeholder="Jump to a server, channel or person, or > for actions" onkeydown={keys} />
    </div>
    <div class="results" id="qs-list" role="listbox" aria-label="Results" hidden={flat.length === 0}>
      {#each sections as sec, si (sec.id)}
        <div class="sec" role="group" aria-labelledby="qs-sec-{sec.id}">
          <div class="head" id="qs-sec-{sec.id}">{sec.label}</div>
          {#each sec.rows as r, ri (r.type === 'place' ? r.place.key : r.action.id)}
            {@const i = starts[si] + ri}
            <!-- an option of the listbox the field drives (aria-activedescendant): the keys stay in the field -->
            <!-- svelte-ignore a11y_click_events_have_key_events -->
            <div class="menu-item row" class:hi={i === active} id={optId(i)} role="option" tabindex="-1" aria-selected={i === active}
              onmousedown={(e) => e.preventDefault()} onpointermove={() => { if (active !== i) active = i }} onclick={() => void choose(i)}>
              {@render lead(r)}
              <span class="name">{label(r)}</span>
              {#if r.type === 'place' && r.place.kind === 'voice'}<span class="sr-only">, voice channel, Shift+Enter to join</span>{/if}
              <span class="end">
                {#if r.type === 'place' ? r.place.context : r.action.context}<small class="ctx">{r.type === 'place' ? r.place.context : r.action.context}</small>{/if}
                <!-- shown on the highlighted row, its room kept on every voice row so nothing shifts as the highlight moves -->
                {#if r.type === 'place' && r.place.kind === 'voice'}
                  <button type="button" class="btn btn-quiet join" tabindex="-1" aria-hidden="true"
                    onclick={(e) => { e.stopPropagation(); void choose(i, true) }}>Join</button>
                {/if}
              </span>
            </div>
          {/each}
        </div>
      {/each}
    </div>
    {#if flat.length === 0}
      <p class="empty">{typed ? `Nothing matches “${raw.trim()}”` : 'Type to jump anywhere. Start with > for actions.'}</p>
    {/if}
    <p class="sr-only" role="status" aria-live="polite">{count}</p>
  </div>
</div>

<style>
  /* above the shell's dialogs' layer (PeoplePicker's 30): it's opened over the app, never under a dialog */
  .overlay { position: fixed; inset: 0; z-index: 31; place-items: start center; padding: 14vh 16px 16px; }
  /* one floating surface: the bar, the results under it */
  .launcher { position: relative; width: min(600px, 100%); max-height: calc(86vh - 32px); display: flex; flex-direction: column; background: var(--surface-menu); border: 1px solid var(--line-menu); border-radius: var(--radius-xl); box-shadow: var(--shadow-3); overflow: hidden; outline: none; --pop-origin: top center; }
  .bar { flex-shrink: 0; display: flex; align-items: center; gap: 12px; height: 56px; padding: 0 18px; color: var(--text-3); }
  .field { flex: 1; min-width: 0; height: 100%; padding: 0; border: 0; background: transparent; color: var(--text); font: inherit; font-size: 18px; outline: none; }
  .field::placeholder { color: var(--text-3); }
  .results { max-height: 440px; overflow-y: auto; padding: 6px; border-top: 1px solid var(--line-menu); display: flex; flex-direction: column; gap: 6px; }
  .results[hidden] { display: none; }
  .sec { display: flex; flex-direction: column; }
  .head { padding: 6px 10px 4px; color: var(--text-3); font-size: 11px; font-weight: 700; text-transform: uppercase; letter-spacing: 0.04em; }
  .row { min-height: 40px; gap: 10px; font-size: 14px; cursor: pointer; }
  .lead { width: 20px; height: 20px; flex-shrink: 0; display: grid; place-items: center; color: var(--text-2); }
  .chip { border-radius: var(--radius-xs); color: #fff; font-size: 11px; font-weight: 700; }
  .name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .end { padding-left: 12px; flex-shrink: 0; min-width: 0; max-width: 45%; display: flex; align-items: center; gap: 8px; }
  .end:empty { display: none; }
  .ctx { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--text-3); font-size: 12px; }
  .join { height: 26px; padding: 0 10px; flex-shrink: 0; visibility: hidden; }
  .row.hi .join { visibility: visible; }
  .empty { margin: 0; padding: 14px 18px 16px; border-top: 1px solid var(--line-menu); color: var(--text-3); font-size: 13px; }
</style>
