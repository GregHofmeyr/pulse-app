<script lang="ts">
  // Server settings → Members (#418): everyone in the server, with their avatar, name and when they joined, oldest
  // first (as the server lists them), the owner marked. Nothing to click on a row but the avatar's card: a person's
  // actions are on their right-click menu (ContextMenu / Shift+F10 on the avatar from the keyboard), the same menu as
  // the member list's, so the owner finds Remove from server and Make owner there. Under them, for the owner only, the
  // people blocked from rejoining (#418: removed three times within 48 hours), with when; Allow back is on their
  // right-click menu. Asked of the server when the pane opens, and again when the member list or the owner changes (a
  // removal is what blocks someone), and after an Allow back.
  import Avatar from './Avatar.svelte'
  import Name from './Name.svelte'
  import PersonMenu from './PersonMenu.svelte'
  import { app } from '../lib/store.svelte'
  import { api, errorText } from '../lib/tauri'
  import { displayName } from '../lib/selectors'
  import { formatMemberSince } from '../lib/profileFormat'
  import { personMenuAt, type PersonMenuAt } from '../lib/participantMenu'
  import type { Server } from '../lib/protocol/Server'
  import type { ServerBlock } from '../lib/protocol/ServerBlock'

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

  // --- blocked from rejoining (#418), the owner's view ---
  const owner = $derived(server.owner_id === app.state.me?.id)
  let blocks = $state<ServerBlock[]>([])
  let blockedMenu = $state<PersonMenuAt | null>(null)
  let error = $state('')
  let asked = 0
  async function loadBlocks() {
    const ask = ++asked
    try {
      const got = await api.listServerBlocks(server.id)
      if (ask === asked) blocks = got ?? []
    } catch (e) {
      if (ask === asked) error = errorText(e)
    }
  }
  $effect(() => {
    // read: a removal (the member count) or a hand-over asks again
    void (app.state.members[server.id] ?? []).length
    if (owner) void loadBlocks()
    else blocks = []
  })
  const blocked = $derived(
    blocks.map((b) => ({ id: b.user_id, name: displayName(app.state, server.id, b.user_id), since: formatMemberSince(b.blocked_at) })),
  )
  function openBlockedMenu(userId: string, e: MouseEvent | KeyboardEvent, row: HTMLElement) {
    blockedMenu = personMenuAt(e, userId, server.id, row, { row: true }) ?? blockedMenu
  }
  async function allowBack(userId: string) {
    error = ''
    try {
      await api.allowBack(server.id, userId)
      blocks = blocks.filter((b) => b.user_id !== userId)
    } catch (e) {
      error = errorText(e)
    }
    void loadBlocks()
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
  {#if owner && blocked.length}
    <h3>Blocked</h3>
    <p class="hint">Removed three times within 48 hours: they can't rejoin until you allow them back. Right-click someone to.</p>
    <ul class="list" aria-label="Blocked from {server.name}">
      {#each blocked as b (b.id)}
        <li><div class="row hover-row" role="presentation" data-blocked={b.id} onpointerenter={() => (hovered = b.id)} onpointerleave={() => { if (hovered === b.id) hovered = null }}
          oncontextmenu={(e) => openBlockedMenu(b.id, e, e.currentTarget)} onkeydown={(e) => openBlockedMenu(b.id, e, e.currentTarget)}>
          <Avatar id={b.id} name={b.name} size={32} interactive serverId={server.id} hovered={() => hovered === b.id} />
          <span class="who">
            <span class="nm"><Name userId={b.id} serverId={server.id} text={b.name} /></span>
            <small class="joined">Blocked {b.since}</small>
          </span>
        </div></li>
      {/each}
    </ul>
  {/if}
  {#if error}<p class="err" role="alert">{error}</p>{/if}
</section>
<PersonMenu {menu} onClose={() => (menu = null)} />
<PersonMenu menu={blockedMenu} onAllowBack={allowBack} onClose={() => (blockedMenu = null)} />

<style>
  h3 { margin: 8px 0 0; font-size: 13px; font-weight: 600; color: var(--text-2); text-transform: uppercase; letter-spacing: .04em; }
  h3 + .hint { margin-top: -8px; }
  .err { margin: 0; font-size: 13px; color: var(--danger-text); }
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
