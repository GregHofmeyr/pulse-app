<script lang="ts">
  // Your custom status (#152), from the status menu: some text and an optional emoji (a standard one, or a custom one
  // from a server you're in), and when it clears: 30 minutes, 1 hour, 4 hours, today, this week, or never. Your recent
  // ones fill the form with one click. The server sanitises the text and keeps the timer (it clears even with every
  // app closed); the length is counted here as it counts it, in characters.
  import { onMount, tick, untrack } from 'svelte'
  import { modal } from '../lib/actions'
  import { app, clock } from '../lib/store.svelte'
  import { api, errorText } from '../lib/tauri'
  import { reactionKey, type Std } from '../lib/emoji'
  import { CLEAR_AFTER, CUSTOM_STATUS_MAX, clearAfterUntil, cleanText, myCustomStatus, sameStatus, statusWords, textLength } from '../lib/customStatus'
  import Icon from './Icon.svelte'
  import EmojiPopover from './EmojiPopover.svelte'
  import StatusEmoji from './StatusEmoji.svelte'
  import CustomStatusLine from './CustomStatusLine.svelte'
  import type { CustomEmoji } from '../lib/protocol/CustomEmoji'
  import type { CustomStatus } from '../lib/protocol/CustomStatus'

  let { onClose }: { onClose: () => void } = $props()

  const current = $derived(myCustomStatus(app.state, clock.nowIso))
  const until = $derived(current ? app.state.myCustomStatus.until : null)
  const title = untrack(() => (current ? 'Edit your custom status' : 'Set a custom status'))
  const recent = $derived(app.state.myCustomStatus.recent.filter((r) => !current || !sameStatus(r, current)))

  let text = $state(untrack(() => current?.text ?? ''))
  let emoji = $state<string | null>(untrack(() => current?.emoji ?? null))
  // A timed status being edited keeps its end unless you pick another; a new one clears at the end of today, as
  // Discord's does.
  const KEEP = 'keep'
  let clearAfter = $state(untrack(() => (current ? (until ? KEEP : 'never') : 'today')))
  const keepLabel = $derived(until ? `Keep (clears ${new Date(until).toLocaleString([], { weekday: 'short', hour: 'numeric', minute: '2-digit' })})` : '')

  let busy = $state(false)
  let error = $state('')
  const length = $derived(textLength(cleanText(text)))
  const problem = $derived(length > CUSTOM_STATUS_MAX ? `Up to ${CUSTOM_STATUS_MAX} characters (${length} now).` : null)
  const empty = $derived(length === 0 && !emoji)

  // the text takes the focus (the dialog's own focus lands on its first control, the emoji button)
  let input = $state<HTMLInputElement>()
  onMount(() => void tick().then(() => input?.focus()))

  let emojiBtn = $state<HTMLButtonElement>()
  let picking = $state(false)
  function picked(e: Std | CustomEmoji) {
    emoji = reactionKey(e)
    picking = false
    emojiBtn?.focus()
  }

  function fill(r: CustomStatus) {
    text = r.text
    emoji = r.emoji
    error = ''
  }

  async function save() {
    if (busy || problem || empty) return
    busy = true
    error = ''
    try {
      const end = clearAfter === KEEP ? until : clearAfterUntil(CLEAR_AFTER.find((c) => c.id === clearAfter) ?? CLEAR_AFTER[3], new Date())
      await api.setCustomStatus(cleanText(text), emoji, end)
      onClose()
    } catch (e) {
      error = errorText(e)
    } finally {
      busy = false
    }
  }
  async function clear() {
    if (busy) return
    busy = true
    error = ''
    try {
      await api.clearCustomStatus()
      onClose()
    } catch (e) {
      error = errorText(e)
    } finally {
      busy = false
    }
  }
  const cancel = () => {
    if (!busy) onClose()
  }
  const customName = (id: string) => app.state.customEmoji[id]?.name
</script>

<div class="overlay modal-overlay fade-in">
  <div class="backdrop modal-backdrop" role="presentation" data-backdrop onclick={cancel}></div>
  <div class="modal pop-in" role="dialog" aria-modal="true" aria-label={title} tabindex="-1" use:modal={cancel}>
    <h1 class="modal-title">{title}</h1>
    <form onsubmit={(e) => { e.preventDefault(); void save() }}>
      <div class="field">
        <button bind:this={emojiBtn} type="button" class="emoji-btn" data-menu-toggle aria-label={emoji ? 'Change emoji' : 'Pick an emoji'} data-tip={emoji ? 'Change emoji' : 'Pick an emoji'}
          disabled={busy} onclick={() => (picking = !picking)}>
          {#if emoji}<StatusEmoji {emoji} size={20} />{:else}<Icon name="smile" size={20} />{/if}
        </button>
        <input bind:this={input} class="input" aria-label="Custom status" aria-invalid={!!problem} bind:value={text} placeholder="What's happening?" disabled={busy} />
        {#if emoji || text}
          <button type="button" class="wipe" aria-label="Clear the text and emoji" data-tip="Clear" disabled={busy} onclick={() => { text = ''; emoji = null }}><Icon name="x" size={14} /></button>
        {/if}
      </div>
      {#if problem}<p class="err" role="alert">{problem}</p>{:else if error}<p class="err" role="alert">{error}</p>{/if}
      <label class="clear-after">
        <span>Clear after</span>
        <select class="input input-sm" bind:value={clearAfter} disabled={busy}>
          {#if until}<option value={KEEP}>{keepLabel}</option>{/if}
          {#each CLEAR_AFTER as c (c.id)}<option value={c.id}>{c.label}</option>{/each}
        </select>
      </label>
      {#if recent.length}
        <div class="recent" role="group" aria-label="Recent">
          <div class="lbl">Recent</div>
          {#each recent as r (`${r.emoji ?? ''} ${r.text}`)}
            <button type="button" class="menu-item" aria-label="Use {statusWords(r, customName)}" disabled={busy} onclick={() => fill(r)}><CustomStatusLine status={r} size={16} /></button>
          {/each}
        </div>
      {/if}
      <div class="actions">
        {#if current}<button type="button" class="btn btn-lg btn-secondary reset" disabled={busy} onclick={() => void clear()}>Clear status</button>{/if}
        <button type="button" class="btn btn-lg btn-secondary" disabled={busy} onclick={cancel}>Cancel</button>
        <button type="submit" class="btn btn-lg btn-primary" disabled={busy || !!problem || empty}>Save</button>
      </div>
    </form>
    {#if picking && emojiBtn}
      <!-- inside the dialog, so it isn't made inert behind it -->
      <EmojiPopover anchor={emojiBtn} onPick={picked} onClose={() => (picking = false)} />
    {/if}
  </div>
</div>

<style>
  /* above the profile card and full profile it opens from, like the nickname dialog (stacking.test.ts) */
  .overlay { position: fixed; inset: 0; z-index: 70; }
  .modal { width: min(420px, 100%); }
  form { display: flex; flex-direction: column; gap: 12px; }
  .field { display: flex; align-items: center; gap: 8px; }
  .field .input { flex: 1; min-width: 0; }
  .emoji-btn, .wipe { display: grid; place-items: center; flex-shrink: 0; border: 0; border-radius: var(--radius-sm); background: var(--bg-3); color: var(--text-2); cursor: pointer; }
  .emoji-btn { width: 38px; height: 38px; }
  .wipe { width: 28px; height: 28px; background: none; }
  .emoji-btn:hover:not(:disabled), .wipe:hover:not(:disabled) { background: var(--bg-4); color: var(--text); }
  .clear-after { display: flex; align-items: center; justify-content: space-between; gap: 12px; font-size: 14px; color: var(--text-2); }
  .recent { display: flex; flex-direction: column; gap: 2px; }
  .recent .menu-item { min-width: 0; }
  .recent :global(.custom-status) { font-size: 13px; color: var(--text); }
  .lbl { padding: 0 2px 4px; font-size: 11px; font-weight: 600; letter-spacing: .06em; text-transform: uppercase; color: var(--text-3); }
  .err { margin: 0; font-size: 13px; color: var(--danger-text); }
  .actions { display: flex; justify-content: flex-end; gap: 8px; }
  .reset { margin-right: auto; }
</style>
