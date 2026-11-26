<script lang="ts">
  // A day field (#643) for Plans, Send later and Scheduled, in place of the native date input (which looks different
  // on every OS): a button the size of an `input-sm` showing the day, opening the shared DatePicker. The value is the
  // same 'YYYY-MM-DD' the native input wrote. Enter or Space on it opens the picker (and stays out of the form's own
  // Enter); other keys go to `onkeydown`.
  import DatePicker from './DatePicker.svelte'
  import Icon from './Icon.svelte'
  import { mediumDay, parseDay, toDay } from '../lib/calendar'

  let { value = $bindable(''), label, min = null, max = null, button = $bindable(), onkeydown }: {
    /** The day ('YYYY-MM-DD'), or '' for none. */
    value?: string
    /** What it's for ("Day"): the button's and the picker's name. */
    label: string
    min?: string | null
    max?: string | null
    button?: HTMLButtonElement
    onkeydown?: (e: KeyboardEvent) => void
  } = $props()

  let open = $state(false)
  let today = $state(toDay(new Date()))
  const shown = $derived(parseDay(value) ? mediumDay(value) : 'Pick a day')

  function toggle() {
    today = toDay(new Date())
    open = !open
  }
  function key(e: KeyboardEvent) {
    if (e.key === 'Enter' || e.key === ' ') e.stopPropagation() // the button's own click opens the picker
    else onkeydown?.(e)
  }
</script>

<button bind:this={button} type="button" class="input input-sm date-field" data-menu-toggle aria-haspopup="dialog" aria-expanded={open}
  aria-label="{label}: {shown}" onclick={toggle} onkeydown={key}>
  <span class="text">{shown}</span>
  <Icon name="calendar" size={14} />
</button>
{#if open && button}
  <DatePicker anchor={button} {label} value={parseDay(value) ? value : null} {min} {max} {today}
    onPick={(d) => (value = d)} onClose={() => (open = false)} />
{/if}

<style>
  .date-field { width: 100%; min-width: 0; box-sizing: border-box; display: flex; align-items: center; justify-content: space-between; gap: 6px; text-align: left; cursor: pointer; }
  .date-field[aria-expanded='true'] { border-color: var(--focus-ring); }
  .text { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .date-field :global(svg) { flex-shrink: 0; color: var(--text-3); }
</style>
