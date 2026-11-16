<script lang="ts">
  // Saved messages (#315), the inbox's Saved tab: a search over notes and text, then each saved message newest first:
  // who and where, a few lines of it and your note. A click opens it where it lives; Note edits the note in place (Save,
  // or Escape to leave it); Remove drops it. One whose message is deleted or out of your reach says so, with your note,
  // and nothing of the message.
  import { relativeTime } from '../lib/conversations'
  import { placeLabel } from '../lib/inbox'
  import { closeInbox } from '../lib/inbox.svelte'
  import { searchBookmarks } from '../lib/bookmarks'
  import { bookmark_limits } from '../lib/bookmarkLimits'
  import { bookmarkAuthor, bookmarks, editBookmarkNote, removeBookmark } from '../lib/bookmarks.svelte'
  import { mentionLookup, showMentions } from '../lib/mentionStyles'
  import { openMessageAt } from '../lib/reminders.svelte'
  import { displayName } from '../lib/selectors'
  import { app, clock } from '../lib/store.svelte'
  import { errorText } from '../lib/tauri'
  import type { MessageBookmark } from '../lib/protocol/MessageBookmark'
  import Avatar from './Avatar.svelte'

  let query = $state('')
  const nowMs = $derived(Date.parse(clock.nowIso))
  const lookup = $derived(mentionLookup(app.state))
  const shown = $derived(searchBookmarks(bookmarks.list, query, bookmarkAuthor))
  let editing = $state<string | null>(null)
  let note = $state('')
  let busy = $state<string | null>(null)
  let errors = $state<Record<string, string>>({})

  const said = (b: MessageBookmark) =>
    b.message ? showMentions(b.message.content, lookup, (id) => displayName(app.state, app.state.channels[b.message!.channel_id]?.server_id ?? null, id)) : ''

  function open(b: MessageBookmark) {
    if (!b.message) return
    closeInbox()
    void openMessageAt(b.message.channel_id, b.message_id, b.message.thread_root_id)
  }
  function edit(b: MessageBookmark) {
    editing = b.id
    note = b.note ?? ''
    errors = { ...errors, [b.id]: '' }
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
  function saveNote(b: MessageBookmark) {
    const n = note.trim()
    if (n === (b.note ?? '')) return void (editing = null)
    void run(b.id, () => editBookmarkNote(b.id, n))
  }
  function noteKey(e: KeyboardEvent, b: MessageBookmark) {
    if (e.key === 'Escape') {
      e.preventDefault()
      e.stopPropagation()
      editing = null
    } else if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
      e.preventDefault()
      saveNote(b)
    }
  }
</script>

{#if bookmarks.list.length}
  <div class="search"><input class="input input-sm" type="search" placeholder="Search saved" aria-label="Search saved messages" bind:value={query} /></div>
{/if}
{#if shown.length}
  <ul class="saved">
    {#each shown as b (b.id)}
      {@const who = bookmarkAuthor(b)}
      <li class="item" class:gone={!b.message}>
        {#if b.message}
          <button type="button" class="open" onclick={() => open(b)}
            aria-label="Saved message from {who} in {placeLabel(app.state, b.message.channel_id, b.message.thread_root_id)}">
            <span class="av" aria-hidden="true"><Avatar id={b.message.author_id ?? ''} name={who} size={32} /></span>
            <span class="text" aria-hidden="true">
              <span class="top"><span class="author">{who}</span><span class="where">{placeLabel(app.state, b.message.channel_id, b.message.thread_root_id)}</span><time datetime={b.message.created_at} data-tip={new Date(b.message.created_at).toLocaleString()}>{relativeTime(b.message.created_at, nowMs)}</time></span>
              <span class="said">{said(b)}</span>
            </span>
          </button>
        {:else}
          <p class="unavailable">This message is no longer available.</p>
        {/if}
        {#if editing === b.id}
          <!-- svelte-ignore a11y_autofocus -->
          <textarea class="input input-sm note-box" bind:value={note} aria-label="Note" rows="2" maxlength={bookmark_limits.NOTE_MAX_CHARS} autofocus onkeydown={(e) => noteKey(e, b)}></textarea>
          <div class="actions">
            <button type="button" class="btn btn-quiet sm" onclick={() => (editing = null)}>Cancel</button>
            <button type="button" class="btn btn-primary sm" disabled={busy === b.id} onclick={() => saveNote(b)}>Save</button>
          </div>
        {:else}
          {#if b.note}<p class="note">{b.note}</p>{/if}
          <div class="actions">
            <button type="button" class="btn btn-quiet sm" disabled={!!busy} onclick={() => edit(b)}>{b.note ? 'Edit note' : 'Add note'}</button>
            <button type="button" class="btn btn-quiet sm danger" disabled={!!busy} onclick={() => void run(b.id, () => removeBookmark(b.id))}>Remove</button>
          </div>
        {/if}
        {#if errors[b.id]}<p class="error" role="alert">{errors[b.id]}</p>{/if}
      </li>
    {/each}
  </ul>
{:else if bookmarks.list.length}
  <p class="none">Nothing saved matches that.</p>
{:else}
  <p class="none">Nothing saved yet. Right-click a message (or its More) and pick Save message.</p>
{/if}

<style>
  .search { padding: 0 8px 4px; }
  .search .input { width: 100%; box-sizing: border-box; }
  .saved { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; }
  .item { display: flex; flex-direction: column; gap: 4px; padding: 0 8px 8px 0; border-radius: var(--radius-sm); font-size: 13px; }
  .item:hover, .item:focus-within { background: var(--bg-4); }
  .item.gone { padding-left: 8px; }
  .open { display: flex; align-items: flex-start; gap: 10px; padding: 8px; border: 0; background: transparent; color: inherit; font: inherit; text-align: left; border-radius: var(--radius-sm); width: 100%; }
  .av { flex-shrink: 0; }
  .text { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px; }
  .top { display: flex; align-items: baseline; gap: 6px; min-width: 0; }
  .author { font-weight: 600; color: var(--text); white-space: nowrap; }
  .where { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--text-3); font-size: 12px; }
  time { flex-shrink: 0; color: var(--text-3); font-size: 12px; }
  .said { color: var(--text-2); white-space: pre-wrap; overflow-wrap: anywhere; display: -webkit-box; -webkit-line-clamp: 3; line-clamp: 3; -webkit-box-orient: vertical; overflow: hidden; }
  .unavailable { margin: 8px 0 0; font-style: italic; color: var(--text-3); }
  .note { margin: 0 0 0 50px; padding-left: 8px; border-left: 2px solid var(--bg-4); color: var(--text-2); white-space: pre-wrap; overflow-wrap: anywhere; }
  .gone .note { margin-left: 0; }
  .note-box { margin-left: 50px; width: calc(100% - 50px); box-sizing: border-box; resize: vertical; font: inherit; }
  .gone .note-box { margin-left: 0; width: 100%; }
  .actions { display: flex; gap: 4px; justify-content: flex-end; }
  .sm { height: 26px; padding: 0 8px; font-size: 12px; }
  .danger { color: var(--danger-text); }
  .error { margin: 0 0 0 8px; color: var(--danger-text); font-size: 12px; }
  .none { margin: 6px 8px; color: var(--text-3); font-size: 13px; }
</style>
