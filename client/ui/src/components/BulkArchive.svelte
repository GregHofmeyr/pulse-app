<script lang="ts">
  // "Archive old builds" (#617): a game (or All games) and a "saved before" (30 days ago, 90 days ago, or a day up to
  // today from the calendar chip's picker), then "Archive matching builds…" asks the server which builds that is (a dry
  // run, covering only builds you may archive), shows them, and
  // archives them once you confirm. Archiving never deletes; each can be restored.
  import { untrack } from 'svelte'
  import DatePicker from './DatePicker.svelte'
  import Icon from './Icon.svelte'
  import Select from './Select.svelte'
  import { modal } from '../lib/actions'
  import { dayStartIso, shortDay } from '../lib/calendar'
  import { clock, ui } from '../lib/store.svelte'
  import { api, errorText } from '../lib/tauri'
  import type { BuildBrief } from '../lib/protocol/BuildBrief'
  import type { Game } from '../lib/protocol/Game'

  let { channelId, games, selected }: {
    channelId: string
    /** The server's games, by name. */
    games: Game[]
    /** The game the view is filtered to: the dropdown starts there and follows it. */
    selected: string | undefined
  } = $props()

  const DAY_MS = 86_400_000
  type Before = { days: 30 | 90 } | { day: string }

  let game = $state<string | null>(untrack(() => selected ?? null))
  $effect(() => {
    game = selected ?? null
  })
  let before = $state<Before>({ days: 30 })
  /** The calendar chip's picker is open. */
  let picking = $state(false)
  let calBtn: HTMLButtonElement | undefined = $state()
  const picked = $derived('day' in before ? `Before ${shortDay(before.day)}` : null)
  /** The time `before` stands for (the start of a picked day, in your time zone; never later than now). */
  function beforeIso(b: Before): string | null {
    if ('days' in b) return new Date(Date.now() - b.days * DAY_MS).toISOString()
    const t = Date.parse(dayStartIso(b.day))
    return Number.isNaN(t) ? null : new Date(Math.min(t, Date.now())).toISOString()
  }
  /** Today in your time zone, as a picker day: the latest it offers. */
  const today = $derived.by(() => {
    const t = new Date(clock.nowIso)
    return `${t.getFullYear()}-${String(t.getMonth() + 1).padStart(2, '0')}-${String(t.getDate()).padStart(2, '0')}`
  })

  let busy = $state(false)
  let error = $state('')
  /** The last preview found nothing for this game and time (cleared when either changes). */
  let none = $state(false)
  let preview = $state<{ builds: BuildBrief[]; body: { game_id: string | null; before: string } } | null>(null)
  $effect(() => {
    void game
    void before
    untrack(() => {
      none = false
      error = ''
    })
  })

  function pickDays(days: 30 | 90) {
    before = { days }
  }

  async function ask() {
    const iso = beforeIso(before)
    if (busy || !iso) return
    busy = true
    error = ''
    const body = { game_id: game, before: iso }
    try {
      const r = await api.bulkArchiveBuilds(channelId, { ...body, dry_run: true })
      if (r.builds.length === 0) none = true
      else preview = { builds: r.builds, body }
    } catch (e) {
      error = errorText(e)
    } finally {
      busy = false
    }
  }

  async function confirm() {
    const p = preview
    if (busy || !p) return
    busy = true
    error = ''
    try {
      const r = await api.bulkArchiveBuilds(channelId, { ...p.body, dry_run: false })
      preview = null
      ui.notice = r.archived === 1 ? 'Archived 1 build' : `Archived ${r.archived} builds`
    } catch (e) {
      error = errorText(e)
    } finally {
      busy = false
    }
  }
  const cancel = () => {
    if (!busy) preview = null
  }

  const SHOWN = 12
  const count = $derived(preview?.builds.length ?? 0)
  const countText = $derived(count === 1 ? '1 build' : `${count} builds`)
</script>

<section class="bulk" aria-labelledby="bulk-h">
  <h3 id="bulk-h" class="menu-label">Archive old builds</h3>
  <div class="field">
    <span class="field-label" aria-hidden="true">Game</span>
    <Select size="sm" label="Game to archive" value={game} onchange={(v) => (game = v)}
      options={[{ value: null, label: 'All games' }, ...games.map((g) => ({ value: g.id, label: g.name }))]} />
  </div>
  <div class="field">
    <span class="field-label" aria-hidden="true">Saved before</span>
    <div class="when" role="group" aria-label="Saved before">
      <button type="button" class="toggle-chip" aria-pressed={'days' in before && before.days === 30} onclick={() => pickDays(30)}>30 days ago</button>
      <button type="button" class="toggle-chip" aria-pressed={'days' in before && before.days === 90} onclick={() => pickDays(90)}>90 days ago</button>
      <button bind:this={calBtn} type="button" class="toggle-chip cal" data-menu-toggle aria-haspopup="dialog" aria-expanded={picking}
        aria-pressed={'day' in before} aria-label={picked ? `Pick a day: ${picked}` : 'Pick a day'} data-tip={picked ?? undefined}
        onclick={() => (picking = !picking)}><Icon name="calendar" size={14} /></button>
    </div>
  </div>
  {#if picking && calBtn}
    <DatePicker anchor={calBtn} label="Saved before" align="end" max={today} {today} value={'day' in before ? before.day : null}
      onPick={(d) => (before = { day: d })} onClose={() => (picking = false)} />
  {/if}
  <button type="button" class="btn btn-secondary go" disabled={busy || none} onclick={ask}>{none ? 'No builds match' : 'Archive matching builds…'}</button>
  <p class="sr-only" role="status">{none ? 'No builds match. Try another game or an earlier day.' : ''}</p>
  {#if error && !preview}<p class="err" role="alert">{error}</p>{/if}
</section>

{#if preview}
  <div class="overlay modal-overlay fade-in">
    <div class="backdrop modal-backdrop" role="presentation" data-backdrop onclick={cancel}></div>
    <div class="modal pop-in" role="dialog" aria-modal="true" aria-label="Archive {countText}" tabindex="-1" use:modal={cancel}>
      <h1 class="modal-title">Archive {countText}?</h1>
      <p>They're hidden from the list but stay searchable, and each can be restored.</p>
      <ul>
        {#each preview.builds.slice(0, SHOWN) as b (b.message_id)}<li>{b.title}</li>{/each}
      </ul>
      {#if count > SHOWN}<p class="more">and {count - SHOWN} more</p>{/if}
      {#if error}<p class="err" role="alert">{error}</p>{/if}
      <div class="actions">
        <button type="button" class="btn btn-lg btn-secondary" disabled={busy} onclick={cancel}>Cancel</button>
        <button type="button" class="btn btn-lg btn-primary" disabled={busy} onclick={confirm}>Archive {countText}</button>
      </div>
    </div>
  </div>
{/if}

<style>
  /* spacing and the left edge come from BuildsPanel (its section rhythm and --inset); every control fills the
     panel's content box, stacked */
  h3 { margin: 0; padding: 0 var(--inset); }
  .field { display: flex; flex-direction: column; gap: 6px; min-width: 0; }
  .field-label { padding-inline: var(--inset); font-size: 12px; font-weight: 600; color: var(--text-3); }
  /* 30 days, 90 days and the calendar chip in one row; the picked day shows in the calendar's tooltip and name */
  .when { display: grid; grid-template-columns: 1fr 1fr auto; gap: 6px; }
  .when .toggle-chip { justify-content: center; min-width: 0; padding: 0 8px; overflow: hidden; text-overflow: ellipsis; }
  .when .cal { width: 36px; padding: 0; }
  .go { width: 100%; min-width: 0; }
  .err { margin: 0; padding-inline: var(--inset); font-size: 12px; color: var(--danger-text); overflow-wrap: anywhere; }
  .overlay { position: fixed; inset: 0; z-index: 30; }
  .modal { width: min(420px, 100%); }
  .modal p { margin: 0; font-size: 14px; color: var(--text-2); }
  ul { margin: 0; padding-left: 18px; max-height: 240px; overflow-y: auto; font-size: 14px; color: var(--text); }
  li { overflow-wrap: anywhere; }
  .more { font-size: 13px; }
  .actions { display: flex; flex-wrap: wrap; justify-content: flex-end; gap: 8px; }
</style>
