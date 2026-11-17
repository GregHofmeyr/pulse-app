<script lang="ts">
  // Custom inbox filters (#307), under the inbox's All and Mentions tabs: a row of your saved filters ("Everything"
  // first) to narrow the list to some people and/or chats, optionally only unread ones; + New filter, and the pencil
  // beside the picked one, open the editor in place (a name, people and chats to tick, found by a search, Only unread,
  // then Save, and Delete for one already saved). Saved on your account, so every device has them; which one is
  // picked is remembered per device.
  import Icon from './Icon.svelte'
  import Avatar from './Avatar.svelte'
  import { activeFilter, inbox, saveFilters, setFilter } from '../lib/inbox.svelte'
  import { filterProblem, MAX_FILTERS, placeLabel } from '../lib/inbox'
  import { displayName, namesOf } from '../lib/selectors'
  import { isNotes } from '../lib/conversations'
  import { app } from '../lib/store.svelte'
  import { errorText } from '../lib/tauri'
  import type { InboxFilter } from '../lib/protocol/InboxFilter'

  const uid = $props.id()
  const filters = $derived(app.state.inboxFilters)
  const picked = $derived(activeFilter())

  /** The editor: a new filter (`original` null) or a saved one being changed. */
  let editing = $state<{ original: string | null; name: string; people: string[]; channels: string[]; unread_only: boolean } | null>(null)
  let query = $state('')
  let error = $state('')
  let saving = $state(false)

  function startNew() {
    editing = { original: null, name: '', people: [], channels: [], unread_only: true }
    query = ''
    error = ''
  }
  function startEdit(f: InboxFilter) {
    editing = { original: f.name, name: f.name, people: [...f.people], channels: [...f.channels], unread_only: f.unread_only }
    query = ''
    error = ''
  }
  const toggle = (list: string[], id: string) => (list.includes(id) ? list.filter((x) => x !== id) : [...list, id])

  const q = $derived(query.trim().toLowerCase())
  // anyone you share a server, DM or group with, by the name you see them by
  const people = $derived.by(() => {
    const s = app.state
    const ids = new Set([...Object.values(s.members).flatMap((ms) => ms.map((m) => m.user.id)), ...Object.values(s.dmMembers).flat()])
    ids.delete(s.me?.id ?? '')
    return [...ids]
      .map((id) => ({ id, name: displayName(s, null, id), all: [s.people[id]?.user.username ?? '', ...namesOf(s, null, id)] }))
      .sort((a, b) => a.name.localeCompare(b.name))
  })
  // every chat an item could be in: server chats (not archived) and your conversations
  const chats = $derived(
    Object.values(app.state.channels)
      .filter((c) => !c.archived_at && !isNotes(app.state, c.id))
      .map((c) => ({ id: c.id, label: placeLabel(app.state, c.id, null) }))
      .sort((a, b) => a.label.localeCompare(b.label)),
  )
  const shownPeople = $derived(editing ? people.filter((p) => editing!.people.includes(p.id) || !q || p.all.some((n) => n.toLowerCase().includes(q))) : [])
  const shownChats = $derived(editing ? chats.filter((c) => editing!.channels.includes(c.id) || !q || c.label.toLowerCase().includes(q)) : [])

  async function save() {
    if (!editing || saving) return
    const e = editing
    const others = filters.filter((f) => f.name !== e.original)
    const next: InboxFilter = { name: e.name.trim(), people: e.people, channels: e.channels, unread_only: e.unread_only }
    error = filterProblem(next, others)
    if (error) return
    const list = e.original === null ? [...filters, next] : filters.map((f) => (f.name === e.original ? next : f))
    saving = true
    try {
      await saveFilters(list)
      editing = null
      setFilter(next.name)
    } catch (err) {
      error = errorText(err)
    } finally {
      saving = false
    }
  }
  async function remove() {
    if (!editing || editing.original === null || saving) return
    const gone = editing.original
    saving = true
    try {
      await saveFilters(filters.filter((f) => f.name !== gone))
      editing = null
      if (inbox.filter === gone) setFilter(null)
    } catch (err) {
      error = errorText(err)
    } finally {
      saving = false
    }
  }
</script>

<div class="filters" role="group" aria-label="Filters">
  <button type="button" class="chip" aria-pressed={!picked} onclick={() => setFilter(null)}>Everything</button>
  {#each filters as f (f.name)}
    <button type="button" class="chip" aria-pressed={picked?.name === f.name} onclick={() => setFilter(f.name)}>{f.name}</button>
    {#if picked?.name === f.name}
      <button type="button" class="edit-btn" aria-label="Edit filter {f.name}" data-tip="Edit filter" onclick={() => startEdit(f)}><Icon name="pencil" size={12} /></button>
    {/if}
  {/each}
  {#if filters.length < MAX_FILTERS}
    <button type="button" class="chip add" onclick={startNew}><Icon name="plus" size={12} /> New filter</button>
  {/if}
</div>

{#if editing}
  {@const e = editing}
  <form class="editor" aria-label={e.original === null ? 'New filter' : `Edit filter ${e.original}`} onsubmit={(ev) => { ev.preventDefault(); void save() }}>
    <input class="input input-sm" bind:value={e.name} maxlength="32" placeholder="Filter name" aria-label="Filter name" />
    <label class="row"><input type="checkbox" role="switch" class="switch" bind:checked={e.unread_only} /> <span>Only unread</span></label>
    <input class="input input-sm" type="search" bind:value={query} placeholder="Find people and chats" aria-label="Find people and chats" />
    <div class="pick">
      <p class="head" id="{uid}-people">From {e.people.length ? `(${e.people.length})` : 'anyone'}</p>
      <ul aria-labelledby="{uid}-people">
        {#each shownPeople as p (p.id)}
          <li><label><input type="checkbox" checked={e.people.includes(p.id)} onchange={() => (e.people = toggle(e.people, p.id))} /><Avatar id={p.id} name={p.name} size={18} /><span class="name">{p.name}</span></label></li>
        {:else}
          <li class="none">Nobody matches.</li>
        {/each}
      </ul>
      <p class="head" id="{uid}-chats">In {e.channels.length ? `(${e.channels.length})` : 'any chat'}</p>
      <ul aria-labelledby="{uid}-chats">
        {#each shownChats as c (c.id)}
          <li><label><input type="checkbox" checked={e.channels.includes(c.id)} onchange={() => (e.channels = toggle(e.channels, c.id))} /><span class="name">{c.label}</span></label></li>
        {:else}
          <li class="none">No chats match.</li>
        {/each}
      </ul>
    </div>
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    <div class="actions">
      {#if e.original !== null}<button type="button" class="btn btn-quiet sm danger" disabled={saving} onclick={() => void remove()}>Delete</button>{/if}
      <span class="grow"></span>
      <button type="button" class="btn btn-quiet sm" onclick={() => (editing = null)}>Cancel</button>
      <button type="submit" class="btn btn-primary sm" disabled={saving}>Save</button>
    </div>
  </form>
{/if}

<style>
  .filters { display: flex; flex-wrap: wrap; align-items: center; gap: 4px; padding: 0 8px; }
  .chip { height: 24px; padding: 0 10px; border: 1px solid var(--bg-4); border-radius: var(--radius-lg); background: transparent; color: var(--text-2); font: inherit; font-size: 12px; display: inline-flex; align-items: center; gap: 4px; transition: background-color var(--dur-fast) var(--ease), color var(--dur-fast) var(--ease); }
  .chip:hover { background: var(--bg-3); color: var(--text); }
  /* the filter in force: the live choice, in the accent */
  .chip[aria-pressed='true'] { background: var(--accent-soft); border-color: transparent; color: var(--accent-text); font-weight: 600; }
  .chip.add { border-style: dashed; color: var(--text-3); }
  .edit-btn { width: 22px; height: 22px; padding: 0; border: 0; border-radius: var(--radius-xs); background: transparent; color: var(--text-3); display: grid; place-items: center; }
  .edit-btn:hover { background: var(--bg-3); color: var(--text); }
  .editor { margin: 0 8px; padding: 10px; border: 1px solid var(--bg-4); border-radius: var(--radius-sm); background: var(--bg-2); display: flex; flex-direction: column; gap: 8px; }
  .row { display: flex; align-items: center; gap: 10px; font-size: 13px; font-weight: 600; }
  .pick { max-height: 200px; overflow-y: auto; display: flex; flex-direction: column; gap: 2px; }
  .head { margin: 4px 0 2px; color: var(--text-3); font-size: 11px; font-weight: 600; letter-spacing: .06em; text-transform: uppercase; }
  ul { margin: 0; padding: 0; list-style: none; display: flex; flex-direction: column; gap: 1px; }
  li label { display: flex; align-items: center; gap: 8px; padding: 3px 4px; border-radius: var(--radius-xs); font-size: 13px; color: var(--text-2); cursor: pointer; }
  li label:hover { background: var(--bg-3); color: var(--text); }
  .name { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .none { color: var(--text-3); font-size: 12px; padding: 3px 4px; }
  .actions { display: flex; align-items: center; gap: 6px; }
  .grow { flex: 1; }
  .sm { height: 26px; padding: 0 10px; font-size: 12px; }
  .danger { color: var(--danger-text); }
  .error { margin: 0; color: var(--danger-text); font-size: 12px; }
</style>
