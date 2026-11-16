<script lang="ts" module>
  type Draft = { title: string; date: string; time: string; place: string; note: string }
  // An unposted plan per chat, kept while the app runs (closing the form doesn't lose it).
  const drafts = new Map<string, Draft>()
  /** For tests: forget every chat's draft. */
  export const forgetPlanDrafts = () => drafts.clear()
</script>

<script lang="ts">
  // The plan form above the message box (#387), the poll composer's sibling: same frame, same place. The title across
  // the top, then the day and time (your own time; the server keeps it in UTC), the place (optional; it gets a
  // forecast when the day is near) and a note (optional), and Post plan, which waits for a title and a time that
  // isn't over. `/plan Braai at kim's` opens it with the title filled in. Its creator's Edit plan opens the same
  // form, filled from the plan, with Save changes.
  import { onMount, tick, untrack } from 'svelte'
  import { clickOutside } from '../lib/actions'
  import { PLAN_LIMITS, defaultStart, fieldsToIso, isoToFields, planReady } from '../lib/plan'
  import { editPlan, postPlan } from '../lib/plans.svelte'
  import { popoverStyle } from '../lib/reactions'
  import { errorText } from '../lib/tauri'
  import type { Message } from '../lib/protocol/Message'

  let { anchor, channelId, editing = null, prefill = null, onClose }: {
    /** What it opens from: the composer's Plan button (`data-menu-toggle`, so a press on it toggles instead), or the
     *  plan's card for an edit. */
    anchor: HTMLElement
    channelId: string
    /** The plan being changed (its creator's Edit plan); none for a new one. */
    editing?: Message | null
    /** What `/plan` was given: the title, replacing the draft's. */
    prefill?: string | null
    onClose: () => void
  } = $props()
  const uid = $props.id()

  const SIZE = { width: 400, height: 470 }
  const style = untrack(() => popoverStyle(anchor.getBoundingClientRect(), SIZE, { width: window.innerWidth, height: window.innerHeight }))

  const start = untrack((): Draft => {
    const p = editing?.plan
    if (p) return { title: p.title, ...isoToFields(p.starts_at), place: p.place ?? '', note: p.note ?? '' }
    const base = drafts.get(channelId) ?? { title: '', ...defaultStart(Date.now()), place: '', note: '' }
    return prefill ? { ...base, title: prefill } : base
  })
  let title = $state(start.title)
  let date = $state(start.date)
  let time = $state(start.time)
  let place = $state(start.place)
  let note = $state(start.note)
  let busy = $state(false)
  let error = $state('')
  let titleField: HTMLInputElement | undefined = $state()

  $effect(() => {
    if (editing) return
    const d = { title, date, time, place, note }
    if (d.title.trim() || d.place.trim() || d.note.trim()) drafts.set(channelId, d)
    else drafts.delete(channelId)
  })

  onMount(() => void tick().then(() => titleField?.focus()))

  const startsAt = $derived(fieldsToIso(date, time))
  const ready = $derived(planReady(title, startsAt, place, note, Date.now()) && !busy)

  async function post() {
    if (!ready || !startsAt) return
    busy = true
    error = ''
    const request = { title: title.trim(), starts_at: startsAt, place: place.trim() || null, note: note.trim() || null }
    try {
      if (editing) await editPlan(editing, request)
      else {
        await postPlan(channelId, request)
        drafts.delete(channelId)
      }
      onClose()
    } catch (e) {
      error = errorText(e)
    } finally {
      busy = false
    }
  }
</script>

<div class="plan-composer pop-in" role="dialog" aria-label={editing ? 'Edit plan' : 'New plan'} use:clickOutside={onClose} {style}>
  <div class="title">
    <input bind:this={titleField} class="input name" bind:value={title} maxlength={PLAN_LIMITS.titleChars} placeholder="What's the plan?"
      aria-label="Title" onkeydown={(e) => { if (e.key === 'Enter' && !e.isComposing) { e.preventDefault(); void post() } }} />
  </div>
  <div class="body">
    <div class="row">
      <label class="field"><span class="menu-label">Day</span><input class="input input-sm" type="date" bind:value={date} aria-label="Day" /></label>
      <label class="field"><span class="menu-label">Time</span><input class="input input-sm" type="time" bind:value={time} aria-label="Time" /></label>
    </div>
    <small class="help">In your own time; everyone sees it in theirs.</small>
    <label class="field"><span class="menu-label">Place</span>
      <input class="input input-sm" bind:value={place} maxlength={PLAN_LIMITS.placeChars} placeholder="Optional: a town or venue" aria-label="Place" aria-describedby="{uid}-place" />
    </label>
    <small class="help" id="{uid}-place">A town or city gets a forecast once the day is within two weeks.</small>
    <label class="field grow"><span class="menu-label">Note</span>
      <textarea class="input input-sm note" bind:value={note} maxlength={PLAN_LIMITS.noteChars} placeholder="Optional" aria-label="Note"></textarea>
    </label>
  </div>
  <div class="foot">
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    <button type="button" class="btn btn-primary btn-lg post" disabled={!ready} onclick={post}>{editing ? 'Save changes' : 'Post plan'}</button>
  </div>
</div>

<style>
  /* the poll composer's frame: the menus' border, radius and shadow round a bg-2 panel */
  .plan-composer { width: 400px; max-width: calc(100vw - 12px); height: 470px; display: flex; flex-direction: column; background: var(--bg-2); border: 1px solid var(--line-menu); border-radius: var(--radius-lg); box-shadow: var(--shadow-3); color: var(--text); overflow: hidden; --pop-origin: bottom left; }
  .title { padding: 12px; background: var(--bg-1); border-bottom: 1px solid var(--line); }
  .name { width: 100%; font-weight: 600; }
  .body { flex: 1; min-height: 0; overflow-y: auto; display: flex; flex-direction: column; gap: 6px; padding: 12px; }
  .row { display: flex; gap: 8px; }
  .row .field { flex: 1; min-width: 0; }
  .field { display: flex; flex-direction: column; gap: 4px; }
  .field .menu-label { padding: 0 2px; }
  .grow { flex: 1; min-height: 90px; }
  .note { flex: 1; resize: none; min-height: 72px; font: inherit; }
  .help { color: var(--text-3); font-size: 11px; margin-bottom: 4px; }
  .foot { display: flex; align-items: center; gap: 8px; padding: 12px; border-top: 1px solid var(--line); }
  .error { flex: 1; margin: 0; color: var(--danger-text); font-size: 12px; }
  .post { margin-left: auto; }
</style>
