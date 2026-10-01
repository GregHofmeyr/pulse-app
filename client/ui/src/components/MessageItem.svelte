<script lang="ts">
  import Icon from './Icon.svelte'
  import { renderMarkdown } from '../lib/markdown'
  import { avatarColor, initial } from '../lib/avatar'
  import type { Message } from '../lib/protocol/Message'

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
  }: {
    message: Message
    name: string
    nameOf: (userId: string | null) => string
    replyTo: Message | null
    grouped: boolean
    mine: boolean
    onReply: () => void
    onEdit: (content: string) => Promise<void>
    onDelete: () => void
  } = $props()

  let editing = $state(false)
  let draft = $state('')
  const time = $derived(new Date(message.created_at).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }))

  function startEdit() {
    draft = message.content
    editing = true
  }
  async function save() {
    const d = draft.trim()
    if (d && d !== message.content) await onEdit(d)
    editing = false
  }
  function key(e: KeyboardEvent) {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault()
      void save()
    } else if (e.key === 'Escape') editing = false
  }
</script>

{#if message.kind === 'system'}
  <div class="system">{message.content} <span class="time">{time}</span></div>
{:else}
  <div class="msg" class:grouped>
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
        {:else}
          <!-- renderMarkdown output is DOMPurify-sanitised (allow-list); the only {@html} in the app. -->
          <div class="content">{@html renderMarkdown(message.content)}</div>
          {#if message.edited_at}<span class="edited">(edited)</span>{/if}
        {/if}
      </div>
      {#if !message.deleted && !editing}
        <div class="actions">
          <button aria-label="Reply" onclick={onReply}><Icon name="reply" size={15} /></button>
          {#if mine}
            <button aria-label="Edit" onclick={startEdit}><Icon name="pencil" size={15} /></button>
            <button aria-label="Delete" onclick={onDelete}><Icon name="trash" size={15} /></button>
          {/if}
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
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
  .msg:hover .actions, .actions:focus-within { display: flex; }
  .actions button { width: 30px; height: 28px; border: 0; background: transparent; color: var(--text-2); display: grid; place-items: center; }
  .actions button:hover { color: var(--text); }
  textarea { width: 100%; resize: vertical; border-radius: 8px; border: 1px solid var(--bg-4); background: var(--bg-1); padding: 8px; font: inherit; }
  .hint { font-size: 11px; color: var(--text-3); }
  .system { padding: 6px 18px 6px 30px; font-size: 13px; color: var(--text-3); }
</style>
