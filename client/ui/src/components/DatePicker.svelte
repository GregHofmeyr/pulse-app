<script lang="ts">
  // A day picker popover (first used by Archive old builds, #617): a month grid, Monday first and always 6 weeks tall,
  // with previous/next month, today marked and days outside min..max disabled. Hung off its button with
  // `placePopover` (kept inside the window, scrolling past `maxHeight`) and moved out to the page (`portal`).
  // Keys (the ARIA date-grid pattern): arrows move a day or a week, Page Up/Down a month, Home/End the week's
  // ends, Enter or Space picks; Escape closes and puts the focus back on the button; a press outside closes.
  import { flushSync, onMount, untrack } from 'svelte'
  import Icon from './Icon.svelte'
  import { clickOutside, portal } from '../lib/actions'
  import { addDays, addMonths, clampDay, monthGrid, parseDay, toDay } from '../lib/calendar'
  import { placePopover } from '../lib/menuPlace'

  let { anchor, label, value = null, min = null, max = null, today = toDay(new Date()), align = 'start', onPick, onClose }: {
    /** The button it hangs off, and gives the focus back to. */
    anchor: HTMLElement
    /** What it's for, read out as the dialog's name. */
    label: string
    /** The picked day ('YYYY-MM-DD'), if any: it opens there. */
    value?: string | null
    min?: string | null
    max?: string | null
    today?: string
    /** Lined up with the button's start or end edge. */
    align?: 'start' | 'end'
    onPick: (day: string) => void
    onClose: () => void
  } = $props()

  const WIDTH = 256
  const uid = `dp-${Math.random().toString(36).slice(2, 8)}`
  const WEEKDAYS = [['Mo', 'Monday'], ['Tu', 'Tuesday'], ['We', 'Wednesday'], ['Th', 'Thursday'], ['Fr', 'Friday'], ['Sa', 'Saturday'], ['Su', 'Sunday']]

  /** The day the keyboard is on (in the tab order); the shown month follows it. */
  let focusDay = $state(untrack(() => clampDay(value ?? today, min, max)))
  /** The shown month, as its first day. */
  let view = $state(untrack(() => focusDay.slice(0, 8) + '01'))
  const shown = $derived(parseDay(view)!)
  const weeks = $derived(monthGrid(shown.getFullYear(), shown.getMonth()))
  const monthName = $derived(new Intl.DateTimeFormat('en-GB', { month: 'long', year: 'numeric' }).format(shown))
  const prevOff = $derived(!!min && addDays(view, -1) < min)
  const nextOff = $derived(!!max && addMonths(view, 1) > max)
  const off = (d: string) => (!!min && d < min) || (!!max && d > max)
  const long = (d: string) => {
    const t = parseDay(d)!
    const f = (o: Intl.DateTimeFormatOptions) => new Intl.DateTimeFormat('en-GB', o).format(t)
    return `${f({ weekday: 'long' })} ${t.getDate()} ${f({ month: 'long' })} ${t.getFullYear()}`
  }

  let el: HTMLDivElement | undefined = $state()
  let place = $state('')
  function placeIt() {
    const p = placePopover(anchor.getBoundingClientRect(), { width: WIDTH, height: el?.offsetHeight || 300 }, undefined, align)
    const y = p.below ? `top: ${p.top}px` : `bottom: ${p.bottom}px`
    place = `left: ${p.left}px; ${y}; max-height: ${p.maxHeight}px; --pop-origin: ${p.below ? 'top' : 'bottom'} ${align === 'end' ? 'right' : 'left'}`
  }
  untrack(placeIt)
  const dayEl = (d: string) => el?.querySelector<HTMLButtonElement>(`[data-day="${d}"]`)
  onMount(() => {
    placeIt() // again with its real height
    dayEl(focusDay)?.focus()
    // a scroll outside it would leave it floating away from its button
    const scroll = (e: Event) => {
      if (!(e.target instanceof Node && el?.contains(e.target))) onClose()
    }
    const t = setTimeout(() => document.addEventListener('scroll', scroll, true))
    return () => {
      clearTimeout(t)
      document.removeEventListener('scroll', scroll, true)
    }
  })

  function close(back: boolean) {
    onClose()
    if (back) anchor.focus()
  }
  function pick(d: string) {
    if (off(d)) return
    onPick(d)
    close(true)
  }
  function moveTo(d: string) {
    focusDay = clampDay(d, min, max)
    view = focusDay.slice(0, 8) + '01'
    flushSync()
    dayEl(focusDay)?.focus()
  }
  function month(n: number) {
    view = addMonths(view, n)
    focusDay = clampDay(addMonths(focusDay, n), min, max)
  }
  function key(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.preventDefault()
      e.stopPropagation()
      return close(true)
    }
    if (!(e.target instanceof HTMLElement) || !e.target.dataset.day) return
    const d = focusDay
    const wd = (parseDay(d)!.getDay() + 6) % 7
    const to: Record<string, () => string> = {
      ArrowLeft: () => addDays(d, -1),
      ArrowRight: () => addDays(d, 1),
      ArrowUp: () => addDays(d, -7),
      ArrowDown: () => addDays(d, 7),
      PageUp: () => addMonths(d, -1),
      PageDown: () => addMonths(d, 1),
      Home: () => addDays(d, -wd),
      End: () => addDays(d, 6 - wd),
    }
    if (to[e.key]) {
      e.preventDefault()
      moveTo(to[e.key]())
    } else if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault()
      pick(d)
    }
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div bind:this={el} class="date-picker menu-surface pop-in" role="dialog" aria-label={label} tabindex="-1" style={place}
  use:portal use:clickOutside={(e) => close(e instanceof KeyboardEvent)} onkeydown={key}>
  <div class="head">
    <button type="button" class="icon-btn icon-btn-sm nav" aria-label="Previous month" disabled={prevOff} onclick={() => month(-1)}><Icon name="chevronLeft" size={16} /></button>
    <span class="month" id="{uid}-month" aria-live="polite">{monthName}</span>
    <button type="button" class="icon-btn icon-btn-sm nav" aria-label="Next month" disabled={nextOff} onclick={() => month(1)}><Icon name="chevronRight" size={16} /></button>
  </div>
  <div class="grid" role="grid" aria-labelledby="{uid}-month">
    <div class="row" role="row">
      {#each WEEKDAYS as [short, full] (short)}<span class="wd" role="columnheader" aria-label={full}>{short}</span>{/each}
    </div>
    {#each weeks as week (week[0])}
      <div class="row" role="row">
        {#each week as d (d)}
          <button type="button" class="day" class:out={d.slice(0, 7) !== view.slice(0, 7)} role="gridcell" data-day={d}
            tabindex={d === focusDay ? 0 : -1} disabled={off(d)} aria-label={long(d)} aria-selected={d === value}
            aria-current={d === today ? 'date' : undefined} onclick={() => pick(d)} onfocus={() => (focusDay = d)}>{Number(d.slice(8))}</button>
        {/each}
      </div>
    {/each}
  </div>
</div>

<style>
  .date-picker { position: fixed; z-index: 70; width: 256px; box-sizing: border-box; padding: 8px; gap: 6px; overflow-y: auto; outline: none; }
  .head { display: flex; align-items: center; gap: 4px; }
  .month { flex: 1; min-width: 0; text-align: center; font-size: 13px; font-weight: 600; color: var(--text); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .nav { flex-shrink: 0; }
  .grid { display: flex; flex-direction: column; gap: 2px; }
  .row { display: grid; grid-template-columns: repeat(7, 1fr); gap: 2px; }
  .wd { height: 24px; display: grid; place-items: center; font-size: 11px; font-weight: 600; color: var(--text-3); }
  .day { height: 30px; min-width: 0; padding: 0; border: 1px solid transparent; border-radius: var(--radius-sm); background: transparent; color: var(--text-2); font: inherit; font-size: 12px; font-variant-numeric: tabular-nums; cursor: pointer; }
  .day.out { color: var(--text-3); }
  .day:hover:not(:disabled) { background: var(--bg-4); color: var(--text); }
  .day[aria-current='date'] { border-color: var(--focus-ring); color: var(--text); font-weight: 600; }
  .day[aria-selected='true'] { background: var(--accent); border-color: transparent; color: var(--on-accent); font-weight: 600; }
  .day:disabled { color: var(--text-3); opacity: .45; cursor: default; }
</style>
