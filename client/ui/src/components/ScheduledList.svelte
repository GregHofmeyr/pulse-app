<script lang="ts">
  // Scheduled messages (#299) as a list: when each goes (and where, in the global list), its text, and why it couldn't
  // go for a failed one. Edit opens the text and time in place (Save, or Escape to leave it); Send now posts it at
  // once; Delete drops it. A failed one given a new time waits again.
  import DateField from './DateField.svelte'
  import { toDay } from '../lib/calendar'
  import { fieldsToIso, isoToFields } from '../lib/plan'
  import { whenLabel, whenReady } from '../lib/schedule'
  import { deleteScheduled, editScheduled, sendScheduledNow, whereTo } from '../lib/scheduled.svelte'
  import { clock } from '../lib/store.svelte'
  import { errorText } from '../lib/tauri'
  import type { ScheduledMessage } from '../lib/protocol/ScheduledMessage'

  let { items, showWhere = false, empty }: {
    items: readonly ScheduledMessage[]
    /** Name each one's chat (the list of all of them). */
    showWhere?: boolean
    /** What an empty list says. */
    empty: string
  } = $props()

  const nowMs = $derived(Date.parse(clock.nowIso))
  let editing = $state<string | null>(null)
  let text = $state('')
  let date = $state('')
  let time = $state('')
  let busy = $state<string | null>(null)
  let errors = $state<Record<string, string>>({})

  function edit(s: ScheduledMessage) {
    editing = s.id
    text = s.content
    ;({ date, time } = isoToFields(s.send_at))
    errors = { ...errors, [s.id]: '' }
  }
  async function run(id: string, action: () => Promise<unknown>) {
    if (busy) return
    busy = id
    errors = { ...errors, [id]: '' }
    try {
      await action()
      if (editing === id) editing = null
    } catch (e) {
      errors = { ...errors, [id]: errorText(e) }
    } finally {
      busy = null
    }
  }
  function save(s: ScheduledMessage) {
    const at = fieldsToIso(date, time)
    const content = text.trim()
    if (!content) return void (errors = { ...errors, [s.id]: 'Write something to send.' })
    const moved = at && at !== s.send_at
    if (moved && !whenReady(at, Date.now())) return void (errors = { ...errors, [s.id]: 'Pick a time ahead, within a year.' })
    if (content === s.content && !moved) return void (editing = null)
    // a failed one keeps its failure until it's given a new time
    void run(s.id, () => editScheduled(s.id, { ...(content !== s.content ? { content } : {}), ...(moved ? { sendAt: at } : {}) }))
  }
  function editKey(e: KeyboardEvent, s: ScheduledMessage) {
    if (e.key === 'Escape') {
      e.preventDefault()
      e.stopPropagation()
      editing = null
    } else if (e.key === 'Enter' && !e.shiftKey && !e.isComposing && e.currentTarget instanceof HTMLTextAreaElement) {
      e.preventDefault()
      save(s)
    }
  }
</script>

{#if items.length}
  <ul class="sched">
    {#each items as s (s.id)}
      <li class="item" class:failed={!!s.failed}>
        <div class="top">
          {#if showWhere}<span class="where">{whereTo(s.channel_id)}</span>{/if}
          <span class="when">{s.failed ? "Couldn't send" : `Sends ${whenLabel(s.send_at, nowMs)}`}</span>
        </div>
        {#if editing === s.id}
          <!-- svelte-ignore a11y_autofocus -->
          <textarea class="input input-sm text" bind:value={text} aria-label="Message" rows="3" autofocus onkeydown={(e) => editKey(e, s)}></textarea>
          <div class="row">
            <DateField bind:value={date} label="Day" min={toDay(new Date())} onkeydown={(e) => editKey(e, s)} />
            <input class="input input-sm" type="time" bind:value={time} aria-label="Time" onkeydown={(e) => editKey(e, s)} />
          </div>
          <div class="actions">
            <button type="button" class="btn btn-quiet sm" onclick={() => (editing = null)}>Cancel</button>
            <button type="button" class="btn btn-primary sm" disabled={busy === s.id} onclick={() => save(s)}>Save</button>
          </div>
        {:else}
          <p class="said">{s.content}</p>
          {#if s.failed}<p class="why">{s.failed}</p>{/if}
          <div class="actions">
            <button type="button" class="btn btn-quiet sm" disabled={!!busy} onclick={() => edit(s)}>Edit</button>
            <button type="button" class="btn btn-quiet sm" disabled={!!busy} onclick={() => void run(s.id, () => sendScheduledNow(s.id))}>Send now</button>
            <button type="button" class="btn btn-quiet sm danger" disabled={!!busy} onclick={() => void run(s.id, () => deleteScheduled(s.id))}>Delete</button>
          </div>
        {/if}
        {#if errors[s.id]}<p class="error" role="alert">{errors[s.id]}</p>{/if}
      </li>
    {/each}
  </ul>
{:else}
  <p class="none">{empty}</p>
{/if}

<style>
  .sched { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; }
  .item { display: flex; flex-direction: column; gap: 4px; padding: 8px; border-radius: var(--radius-sm); font-size: 13px; }
  .item:hover, .item:focus-within { background: var(--bg-4); }
  .item.failed { box-shadow: inset 2px 0 0 var(--danger); }
  .top { display: flex; align-items: baseline; gap: 6px; min-width: 0; }
  .where { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: 600; color: var(--text); }
  .when { flex-shrink: 0; color: var(--text-3); font-size: 12px; }
  .failed .when { color: var(--danger-text); }
  .said { margin: 0; color: var(--text-2); white-space: pre-wrap; overflow-wrap: anywhere; display: -webkit-box; -webkit-line-clamp: 3; line-clamp: 3; -webkit-box-orient: vertical; overflow: hidden; }
  .why { margin: 0; color: var(--danger-text); font-size: 12px; }
  .text { width: 100%; resize: vertical; font: inherit; }
  .row { display: flex; gap: 6px; }
  .row .input, .row :global(.date-field) { flex: 1; min-width: 0; }
  .actions { display: flex; gap: 4px; justify-content: flex-end; }
  .sm { height: 26px; padding: 0 8px; font-size: 12px; }
  .danger { color: var(--danger-text); }
  .error { margin: 0; color: var(--danger-text); font-size: 12px; }
  .none { margin: 6px 8px; color: var(--text-3); font-size: 13px; }
</style>
