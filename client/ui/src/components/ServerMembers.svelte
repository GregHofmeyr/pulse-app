<script lang="ts">
  // Server settings → Members (#418): everyone in the server, with their avatar, name and when they joined, oldest
  // first (as the server lists them), the owner marked. Nothing to click on a row but the avatar's card: a person's
  // actions are on their right-click menu (ContextMenu / Shift+F10 on the avatar from the keyboard), the same menu as
  // the member list's, so the owner finds Remove from server and Make owner there.
  import Avatar from './Avatar.svelte'
  import Name from './Name.svelte'
  import PersonMenu from './PersonMenu.svelte'
  import { app } from '../lib/store.svelte'
  import { displayName } from '../lib/selectors'
  import { formatMemberSince } from '../lib/profileFormat'
  import { personMenuAt, type PersonMenuAt } from '../lib/participantMenu'
  import type { Server } from '../lib/protocol/Server'

  let { server }: { server: Server } = $props()

  const members = $derived(
    (app.state.members[server.id] ?? []).map((m) => ({
      id: m.user.id,
      name: displayName(app.state, server.id, m.user.id),
      joined: m.joined_at ? formatMemberSince(m.joined_at) : null,
    })),
  )
  let hovered = $state<string | null>(null)
  let menu = $state<PersonMenuAt | null>(null)
  function openMenu(userId: string, e: MouseEvent | KeyboardEvent, row: HTMLElement) {
    menu = personMenuAt(e, userId, server.id, row, { row: true }) ?? menu
  }
</script>

<section class="settings-pane">
  <h2>Members</h2>
  <p class="hint">{members.length} {members.length === 1 ? 'person' : 'people'}. Right-click someone for what you can do.</p>
  <ul class="list" aria-label="Members of {server.name}">
    {#each members as m (m.id)}
      <li><div class="row hover-row" role="presentation" data-user={m.id} onpointerenter={() => (hovered = m.id)} onpointerleave={() => { if (hovered === m.id) hovered = null }}
        oncontextmenu={(e) => openMenu(m.id, e, e.currentTarget)} onkeydown={(e) => openMenu(m.id, e, e.currentTarget)}>
        <Avatar id={m.id} name={m.name} size={32} interactive serverId={server.id} hovered={() => hovered === m.id} />
        <span class="who">
          <span class="nm"><Name userId={m.id} serverId={server.id} text={m.name} />{#if m.id === app.state.me?.id}<small> (you)</small>{/if}</span>
          {#if m.joined}<small class="joined">Joined {m.joined}</small>{/if}
        </span>
        {#if m.id === server.owner_id}<span class="owner">Owner</span>{/if}
      </div></li>
    {/each}
  </ul>
</section>
<PersonMenu {menu} onClose={() => (menu = null)} />

<style>
  .hint { margin: -12px 0 0; color: var(--text-3); font-size: 12px; }
  .list { margin: 0; padding: 0; list-style: none; display: flex; flex-direction: column; gap: 2px; }
  .row { min-height: 48px; padding: 6px 10px; border-radius: var(--radius-md); display: flex; align-items: center; gap: 12px; }
  .row:hover { background: var(--bg-2); --ring: var(--bg-2); }
  .who { flex: 1; min-width: 0; display: flex; flex-direction: column; font-size: 14px; font-weight: 500; }
  .nm { min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .nm small { color: var(--text-3); font-weight: 400; }
  .joined { color: var(--text-3); font-size: 12px; font-weight: 400; }
  .owner { flex-shrink: 0; padding: 2px 8px; border-radius: var(--radius-sm); background: var(--bg-3); color: var(--text-2); font-size: 11px; font-weight: 600; letter-spacing: .04em; text-transform: uppercase; }
</style>
