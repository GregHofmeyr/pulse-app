<script lang="ts">
  // Server settings → Quote speakers: plain names linked to members (spec §3.4). Anyone can undo a
  // link or point it at someone else; lines keep their original name either way.
  import { app } from '../lib/store.svelte'
  import { api, errorText } from '../lib/tauri'
  import { suggest } from '../lib/mentions'
  import { displayName } from '../lib/selectors'
  import type { Server } from '../lib/protocol/Server'
  import type { SpeakerLink } from '../lib/protocol/SpeakerLink'

  let { server }: { server: Server } = $props()

  const links = $derived([...(app.state.speakerLinks[server.id] ?? [])].sort((a, b) => a.name.localeCompare(b.name)))
  const members = $derived((app.state.members[server.id] ?? []).map((m) => ({ id: m.user.id, name: m.nickname ?? m.user.username })))
  const nameOf = (id: string) => displayName(app.state, server.id, id)

  let error = $state('')
  // The link being re-pointed, with the same type-ahead as the line menu's "Link to a member…".
  let changing = $state<string | null>(null)
  let query = $state('')
  // Not the member it's already linked to: picking them would change nothing.
  const matches = $derived(suggest(members.filter((m) => m.id !== links.find((l) => l.name_key === changing)?.user_id), query))

  async function attempt(f: () => Promise<unknown>) {
    error = ''
    try {
      await f()
    } catch (e) {
      error = errorText(e)
    }
  }
  function startChange(l: SpeakerLink) {
    changing = l.name_key
    query = ''
  }
  function repoint(l: SpeakerLink, userId: string) {
    changing = null
    void attempt(() => api.linkSpeaker(server.id, { name: l.name, user_id: userId }))
  }
  function findKey(e: KeyboardEvent, l: SpeakerLink) {
    if (e.key === 'Escape') {
      e.preventDefault() // closes the picker, not the dialog
      changing = null
    } else if (e.key === 'Enter' && matches[0]) {
      e.preventDefault()
      repoint(l, matches[0].id)
    }
  }
</script>

<h2>Quote speakers</h2>
<p class="hint">Names in quotes that belong to a member. Undoing a link puts the plain name back on its lines.</p>
{#if error}<p class="error" role="alert">{error}</p>{/if}
{#if links.length === 0}
  <p class="empty">No names are linked yet. Link one from a quote line's menu.</p>
{:else}
  <ul>
    {#each links as l (l.name_key)}
      <li class="link">
        <div class="meta">
          <span><span class="name">{l.name}</span> → {nameOf(l.user_id)}</span>
          <small>linked by {nameOf(l.linked_by)}</small>
        </div>
        {#if changing === l.name_key}
          <div class="pick">
            <!-- svelte-ignore a11y_autofocus -->
            <input type="text" aria-label="Find a member" placeholder="Find a member" autocomplete="off" autofocus bind:value={query} onkeydown={(e) => findKey(e, l)} />
            <div class="matches" role="group" aria-label="Members">
              {#each matches as m (m.id)}
                <button type="button" class="btn btn-secondary" onclick={() => repoint(l, m.id)}>{m.name}</button>
              {:else}
                <span class="none">No one by that name</span>
              {/each}
            </div>
            <button type="button" class="btn btn-secondary" onclick={() => (changing = null)}>Cancel</button>
          </div>
        {:else}
          <button type="button" class="btn btn-secondary" onclick={() => startChange(l)}>Change</button>
          <button type="button" class="btn btn-secondary btn-danger-text" onclick={() => void attempt(() => api.unlinkSpeaker(server.id, l.name_key))}>Undo</button>
        {/if}
      </li>
    {/each}
  </ul>
{/if}

<style>
  h2 { margin: 0 0 8px; font-size: 20px; }
  .hint { margin: 0 0 16px; color: var(--text-3); font-size: 13px; }
  .error { margin: 0 0 12px; color: var(--danger); font-size: 13px; }
  .empty { color: var(--text-3); }
  ul { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; }
  li { display: flex; align-items: center; gap: 8px; padding: 8px 10px; border-radius: 10px; background: var(--bg-0); flex-wrap: wrap; }
  .meta { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .name { font-weight: 600; }
  small { color: var(--text-3); }
  .pick { display: flex; align-items: flex-start; gap: 8px; flex-wrap: wrap; }
  input { height: 32px; padding: 0 10px; border: 1px solid var(--accent); border-radius: 8px; background: var(--bg-2); color: var(--text); outline: none; }
  .matches { display: flex; flex-wrap: wrap; gap: 4px; }
  .none { padding: 6px 4px; color: var(--text-3); font-size: 13px; }
</style>
