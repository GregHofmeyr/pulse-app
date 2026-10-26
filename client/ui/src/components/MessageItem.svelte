<script lang="ts" module>
  /** How long a failed delete/reaction error stays on its message. */
  export const INLINE_ERROR_MS = 5_000
</script>

<script lang="ts">
  import Icon from './Icon.svelte'
  import EmojiPopover from './EmojiPopover.svelte'
  import ReactionRow from './ReactionRow.svelte'
  import { reactionKey, type Std } from '../lib/emoji'
  import { pickAdds, react } from '../lib/reactions'
  import { app } from '../lib/store.svelte'
  import { errorText } from '../lib/tauri'
  import type { CustomEmoji } from '../lib/protocol/CustomEmoji'
  import { renderMessage } from '../lib/markdown'
  import { animateEmojiIn } from '../lib/animate.svelte'
  import { avatarColor, initial } from '../lib/avatar'
  import type { Message } from '../lib/protocol/Message'
  import type { Pending } from '../lib/state'

  let {
    message,
    name,
    nameOf,
    replyTo,
    grouped,
    mine,
    onReply,
    onEdit,
    onDelete,
    mentionNames = [],
    meId = '',
    pending,
    onRetry = () => {},
    onDiscard = () => {},
  }: {
    message: Message
    name: string
    nameOf: (userId: string | null) => string
    replyTo: Message | null
    grouped: boolean
    mine: boolean
    onReply: () => void
    onEdit: (content: string) => Promise<void>
    onDelete: () => Promise<void>
    /** Usernames that may be mentioned here (rendered as pills). */
    mentionNames?: string[]
    meId?: string
    /** Set when `message` is an unsent one (`pendingMessage`): faded, no actions or reactions, status line under it. */
    pending?: Pending
    onRetry?: () => void
    onDiscard?: () => void
  } = $props()

  let picking = $state(false)
  let smileBtn: HTMLButtonElement | undefined = $state()
  let editing = $state(false)
  let draft = $state('')
  let editError = $state('')
  let inlineError = $state('')
  let errTimer: ReturnType<typeof setTimeout> | undefined
  // A reaction can fail after the message has gone from the screen: no timer for a component that's gone.
  let alive = true
  // A failed delete or reaction shows on this message for a few seconds; a newer one restarts the clock.
  function showError(msg: string) {
    if (!alive) return
    clearTimeout(errTimer)
    inlineError = msg
    errTimer = setTimeout(() => (inlineError = ''), INLINE_ERROR_MS)
  }
  $effect(() => () => {
    alive = false
    clearTimeout(errTimer)
  })
  async function del() {
    try {
      await onDelete()
    } catch (e) {
      showError(errorText(e))
    }
  }
  const time = $derived(new Date(message.created_at).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }))

  function startEdit() {
    draft = message.content
    editError = ''
    editing = true
  }
  // a second Enter while the first save is in flight must not send the edit twice
  let saving = false
  async function save() {
    if (saving) return
    const d = draft.trim()
    editError = ''
    if (d && d !== message.content) {
      saving = true
      try {
        await onEdit(d)
      } catch (e) {
        editError = errorText(e) // the box stays open with the text intact
        return
      } finally {
        saving = false
      }
    }
    editing = false
  }
  function pick(e: Std | CustomEmoji, { keep }: { keep: boolean }) {
    if (!keep) picking = false
    const key = reactionKey(e)
    void react(() => app.state, (s) => (app.state = s), message.channel_id, message.id, key, pickAdds(message.reactions, key, keep), meId, showError)
  }
  function key(e: KeyboardEvent) {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault()
      void save()
    } else if (e.key === 'Escape') {
      editing = false
      editError = ''
    }
  }
</script>

{#snippet item()}
{#if message.kind === 'system'}
  <div class="system">{message.content} <span class="time">{time}</span></div>
{:else}
  <div class="msg" class:grouped class:mentioned={!!meId && message.mentions.includes(meId)}>
    {#if replyTo}
      <div class="reply"><span class="hook"></span><strong>{nameOf(replyTo.author_id)}</strong>
        <span class="snippet">{replyTo.deleted ? 'original message deleted' : replyTo.content.slice(0, 120)}</span></div>
    {:else if message.reply_to_id}
      <div class="reply"><span class="hook"></span><span class="snippet">original message not loaded</span></div>
    {/if}
    <div class="row">
      {#if grouped && !replyTo}
        <span class="gutter"></span>
      {:else}
        <span class="av" style:background={avatarColor(message.author_id ?? '')}>{initial(name)}</span>
      {/if}
      <div class="body">
        {#if !grouped || replyTo}<div class="meta"><strong>{name}</strong><span class="time">{time}</span></div>{/if}
        {#if message.deleted}
          <p class="deleted">message deleted</p>
        {:else if editing}
          <textarea bind:value={draft} onkeydown={key} rows="2" aria-label="Edit message"></textarea>
          <small class="hint">Enter to save · Esc to cancel</small>
          {#if editError}<p class="err" role="alert">{editError}</p>{/if}
        {:else}
          <!-- renderMessage output is DOMPurify-sanitised (allow-list, cached per message); the only {@html} in the app. -->
          {@const html = renderMessage(message, mentionNames, app.state.customEmoji)}
          <div class="content" {@attach animateEmojiIn(html)}>{@html html}</div>
          {#if message.edited_at}<span class="edited">(edited)</span>{/if}
        {/if}
        {#if !pending}<ReactionRow {message} {meId} {nameOf} onError={showError} />{/if}
        {#if inlineError}<p class="err" role="alert">{inlineError}</p>{/if}
      </div>
      {#if !pending && !message.deleted && !editing}
        <div class="actions" class:open={picking}>
          <button bind:this={smileBtn} aria-label="Add reaction" data-menu-toggle onclick={() => (picking = !picking)}><Icon name="smile" size={15} /></button>
          <button aria-label="Reply" onclick={onReply}><Icon name="reply" size={15} /></button>
          {#if mine}
            <button aria-label="Edit" onclick={startEdit}><Icon name="pencil" size={15} /></button>
            <button aria-label="Delete" onclick={() => void del()}><Icon name="trash" size={15} /></button>
          {/if}
        </div>
        {#if picking && smileBtn}
          <EmojiPopover anchor={smileBtn} onPick={pick} onClose={() => (picking = false)} />
        {/if}
      {/if}
    </div>
  </div>
{/if}
{/snippet}

{#if pending}
  <div class="pend">
    <div class="unsent" style:opacity={0.6}>{@render item()}</div>
    <div class="status" class:failed={pending.status === 'failed'}>
      {#if pending.status === 'failed'}
        <span class="fail">Failed to send</span>
        <button onclick={onRetry}>Retry</button>
        <button onclick={onDiscard}>Discard</button>
      {:else}
        <span>Sending…</span>
      {/if}
    </div>
  </div>
{:else}
  {@render item()}
{/if}

<style>
  .status { display: flex; gap: 10px; align-items: baseline; padding: 0 18px 4px 72px; font-size: 12px; color: var(--text-3); }
  .status .fail { color: var(--danger); }
  .status button { border: 0; background: none; padding: 0; font-size: 12px; color: var(--text-2); text-decoration: underline; }
  .msg { padding: 8px 18px 2px; position: relative; }
  .msg.grouped { padding-top: 1px; }
  .msg:hover { background: rgba(255, 255, 255, .02); }
  .row { display: flex; gap: 14px; }
  .av { width: 40px; height: 40px; border-radius: 50%; color: #fff; font-weight: 700; display: grid; place-items: center; flex-shrink: 0; }
  .gutter { width: 40px; flex-shrink: 0; }
  .body { flex: 1; min-width: 0; }
  .meta { display: flex; align-items: baseline; gap: 8px; margin-bottom: 2px; }
  .meta strong { font-size: 14.5px; }
  .time { font-size: 11px; color: var(--text-3); }
  .content { color: #dcdee5; line-height: 1.5; overflow-wrap: anywhere; }
  .content :global(p) { margin: 0; }
  .content :global(code) { font-family: 'JetBrains Mono', ui-monospace, monospace; font-size: 12.5px; background: #17181c; border: 1px solid var(--line); border-radius: 6px; padding: 1px 6px; }
  .content :global(pre) { background: #17181c; border-radius: 8px; padding: 10px; overflow-x: auto; }
  .content :global(pre code) { border: 0; padding: 0; }
  .content :global(a) { color: var(--accent); }
  .content :global(blockquote) { margin: 4px 0; padding-left: 10px; border-left: 3px solid var(--bg-4); color: var(--text-2); }
  .edited { font-size: 11px; color: var(--text-3); }
  .deleted { margin: 0; font-style: italic; color: var(--text-3); }
  .reply { display: flex; align-items: center; gap: 6px; padding-left: 20px; margin-bottom: 4px; font-size: 13px; color: var(--text-3); min-width: 0; }
  .reply strong { color: var(--text-2); }
  .hook { width: 28px; height: 10px; margin-top: 10px; border-left: 2px solid #3a3c44; border-top: 2px solid #3a3c44; border-top-left-radius: 6px; flex-shrink: 0; }
  .snippet { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .actions { position: absolute; top: -10px; right: 18px; display: none; background: var(--bg-3); border: 1px solid var(--bg-4); border-radius: 8px; }
  .msg:hover .actions, .actions:focus-within, .actions.open { display: flex; }
  .actions button { width: 30px; height: 28px; border: 0; background: transparent; color: var(--text-2); display: grid; place-items: center; }
  .actions button:hover { color: var(--text); }
  textarea { width: 100%; resize: vertical; border-radius: 8px; border: 1px solid var(--bg-4); background: var(--bg-1); padding: 8px; font: inherit; }
  .err { margin: 2px 0 0; font-size: 12px; color: var(--danger); }
  .hint { font-size: 11px; color: var(--text-3); }
  .system { padding: 6px 18px 6px 30px; font-size: 13px; color: var(--text-3); }
  .msg.mentioned { background: rgba(139, 156, 255, .08); box-shadow: inset 2px 0 0 var(--accent); }
  .content :global(img.emoji) { width: 1.375em; height: 1.375em; vertical-align: -0.3em; object-fit: contain; }
  .content :global(img.emoji.big) { width: 48px; height: 48px; }
  .content :global(.emoji-gone) { color: var(--text-3); background: var(--bg-3); border-radius: 4px; padding: 0 4px; font-size: .9em; }
  .content :global(.emoji-gone.big) { font-size: 1.25em; }
  .content :global(.mention) { color: var(--accent); background: rgba(139, 156, 255, .15); border-radius: 4px; padding: 0 3px; font-weight: 600; }
</style>
