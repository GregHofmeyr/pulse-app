<script lang="ts" module>
  type Draft = { question: string; options: string[]; multi: boolean; anonymous: boolean; ends: PollDuration }
  // An unposted poll per chat, kept while the app runs (closing the composer doesn't lose it).
  const drafts = new Map<string, Draft>()
  /** For tests: forget every chat's draft. */
  export const forgetPollDrafts = () => drafts.clear()
</script>

<script lang="ts">
  // The poll composer above the message box (#157), the wheel popover's sibling: same frame, same place. A question
  // across the top, the Multiple choice and Anonymous switches and Ends in under it, then the options (two rows to
  // start, + Add option up to ten, removable past two) and Post poll, which waits for a question and two different
  // options. `/poll Question? a, b` opens it filled in.
  import { onMount, tick, untrack } from 'svelte'
  import Icon from './Icon.svelte'
  import Segmented from './Segmented.svelte'
  import { clickOutside } from '../lib/actions'
  import { POLL_ENDS, POLL_LIMITS, pollReady } from '../lib/poll'
  import { postPoll } from '../lib/polls.svelte'
  import { popoverStyle } from '../lib/reactions'
  import { errorText } from '../lib/tauri'
  import type { PollDuration } from '../lib/protocol/PollDuration'

  let { anchor, channelId, prefill = null, onClose }: {
    /** The composer's poll button; it has `data-menu-toggle`, so a press on it toggles instead. */
    anchor: HTMLElement
    channelId: string
    /** What `/poll` was given: it replaces the draft. */
    prefill?: { question: string; options: string[] } | null
    onClose: () => void
  } = $props()
  const uid = $props.id()

  const SIZE = { width: 420, height: 480 }
  const style = untrack(() => popoverStyle(anchor.getBoundingClientRect(), SIZE, { width: window.innerWidth, height: window.innerHeight }))

  const start = untrack((): Draft => {
    const kept = drafts.get(channelId)
    const base = kept ?? { question: '', options: ['', ''], multi: false, anonymous: false, ends: 'day' as PollDuration }
    if (!prefill) return base
    const options = prefill.options.slice(0, POLL_LIMITS.maxOptions)
    while (options.length < POLL_LIMITS.minOptions) options.push('')
    return { ...base, question: prefill.question, options }
  })
  let question = $state(start.question)
  let options = $state<string[]>([...start.options])
  let multi = $state(start.multi)
  let anonymous = $state(start.anonymous)
  let ends = $state<PollDuration>(start.ends)
  let busy = $state(false)
  let error = $state('')
  let questionField: HTMLInputElement | undefined = $state()
  let list: HTMLElement | undefined = $state()

  $effect(() => {
    const d = { question, options: [...options], multi, anonymous, ends }
    if (d.question.trim() || d.options.some((o) => o.trim())) drafts.set(channelId, d)
    else drafts.delete(channelId)
  })

  onMount(() => {
    // a filled-in question goes straight on to the first empty option
    const empty = question.trim() ? options.findIndex((o) => !o.trim()) : -1
    void tick().then(() => (empty >= 0 ? focusOption(empty) : questionField?.focus()))
  })

  const ready = $derived(pollReady(question, options) && !busy)
  const full = $derived(options.length >= POLL_LIMITS.maxOptions)

  function focusOption(i: number) {
    list?.querySelectorAll<HTMLInputElement>('input')[i]?.focus()
  }

  async function addOption() {
    if (full) return
    options = [...options, '']
    await tick()
    focusOption(options.length - 1)
  }

  async function removeOption(i: number) {
    if (options.length <= POLL_LIMITS.minOptions) return
    options = options.filter((_, j) => j !== i)
    await tick()
    focusOption(Math.min(i, options.length - 1))
  }

  // Enter moves on: from the question to the first option, from an option to the next (a new one after the last)
  function next(e: KeyboardEvent, i: number) {
    if (e.key !== 'Enter' || e.isComposing) return
    e.preventDefault()
    if (i + 1 < options.length) focusOption(i + 1)
    else if (!full) void addOption()
    else if (ready) void post()
  }

  async function post() {
    if (!ready) return
    busy = true
    error = ''
    try {
      await postPoll(channelId, { question: question.trim(), options: options.map((o) => o.trim()), multi, anonymous, ends_in: ends })
      drafts.delete(channelId)
      onClose()
    } catch (e) {
      error = errorText(e)
    } finally {
      busy = false
    }
  }
</script>

<div class="poll-composer pop-in" role="dialog" aria-label="New poll" use:clickOutside={onClose} {style}>
  <div class="title">
    <input bind:this={questionField} class="input question" bind:value={question} maxlength={POLL_LIMITS.questionChars} placeholder="Ask a question"
      aria-label="Question" onkeydown={(e) => { if (e.key === 'Enter' && !e.isComposing) { e.preventDefault(); focusOption(0) } }} />
  </div>
  <div class="body">
    <div class="toggles">
      <label class="toggle"><input type="checkbox" role="switch" class="switch" bind:checked={multi} /><span>Multiple choice</span></label>
      <label class="toggle"><input type="checkbox" role="switch" class="switch" bind:checked={anonymous} aria-describedby="{uid}-anon" /><span>Anonymous</span></label>
    </div>
    <small class="help" id="{uid}-anon">{anonymous ? 'Nobody sees who voted for what, you included.' : 'Everyone sees who voted for what.'}</small>
    <div class="menu-label">Ends in</div>
    <Segmented label="Ends in" options={POLL_ENDS.map((o) => ({ value: o.value, label: o.label }))} value={ends} onchange={(v) => (ends = v)} />
    <div class="menu-label">Options</div>
    <ol class="options" bind:this={list}>
      {#each options as _, i (i)}
        <li>
          <input class="input input-sm" bind:value={options[i]} maxlength={POLL_LIMITS.optionChars} placeholder="Option {i + 1}" aria-label="Option {i + 1}"
            onkeydown={(e) => next(e, i)} />
          {#if options.length > POLL_LIMITS.minOptions}
            <button type="button" class="icon-btn" aria-label="Remove option {i + 1}" onclick={() => void removeOption(i)}><Icon name="x" size={16} /></button>
          {/if}
        </li>
      {/each}
    </ol>
    {#if !full}
      <button type="button" class="add" onclick={() => void addOption()}><Icon name="plus" size={14} /> Add option</button>
    {/if}
  </div>
  <div class="foot">
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    <button type="button" class="btn btn-primary btn-lg post" disabled={!ready} onclick={post}>Post poll</button>
  </div>
</div>

<style>
  /* the wheel popover's frame: the menus' border, radius and shadow round a bg-2 panel */
  .poll-composer { width: 420px; max-width: calc(100vw - 12px); height: 480px; display: flex; flex-direction: column; background: var(--bg-2); border: 1px solid var(--line-menu); border-radius: var(--radius-lg); box-shadow: var(--shadow-3); color: var(--text); overflow: hidden; --pop-origin: bottom left; }
  /* the question is the title bar, on bg-1 like the wheel popover's stage */
  .title { padding: 12px; background: var(--bg-1); border-bottom: 1px solid var(--line); }
  .question { width: 100%; font-weight: 600; }
  .body { flex: 1; min-height: 0; overflow-y: auto; display: flex; flex-direction: column; gap: 8px; padding: 12px; }
  .body .menu-label { padding: 4px 2px 0; }
  .toggles { display: flex; flex-wrap: wrap; gap: 8px 20px; }
  .toggle { display: inline-flex; align-items: center; gap: 10px; font-size: 13px; cursor: pointer; }
  .help { color: var(--text-3); font-size: 11px; }
  .options { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
  .options li { display: flex; align-items: center; gap: 6px; }
  .options .input { flex: 1; min-width: 0; }
  .add { align-self: flex-start; display: inline-flex; align-items: center; gap: 6px; min-height: 30px; padding: 0 8px; border: 0; border-radius: var(--radius-sm); background: transparent; color: var(--accent-text); font: inherit; font-size: 13px; font-weight: 600; cursor: pointer; }
  .add:hover { background: var(--bg-3); }
  .foot { display: flex; align-items: center; gap: 8px; padding: 12px; border-top: 1px solid var(--line); }
  .error { flex: 1; margin: 0; color: var(--danger-text); font-size: 12px; }
  .post { margin-left: auto; }
</style>
