<script lang="ts">
  // Server settings → Bans (#80), the owner's: who's banned, newest first, with when. A banned person can't join again,
  // even by invite. Unban is on their right-click menu (ContextMenu / Shift+F10 from the keyboard). Asked when the pane opens and again on BansChanged (a ban or unban on any of the owner's devices).
  import { untrack } from 'svelte'
  import Avatar from './Avatar.svelte'
  import Name from './Name.svelte'
  import PersonMenu from './PersonMenu.svelte'
  import { app } from '../lib/store.svelte'
  import { api, errorText } from '../lib/tauri'
  import { displayName } from '../lib/selectors'
  import { formatMemberSince } from '../lib/profileFormat'
  import { personMenuAt, type PersonMenuAt } from '../lib/participantMenu'
  import type { Server } from '../lib/protocol/Server'
  import type { ServerBan } from '../lib/protocol/ServerBan'

  let { server }: { server: Server } = $props()

  let bans = $state<ServerBan[] | null>(null)
  let error = $state('')
  let asked = 0
  async function load() {
    const ask = ++asked
    try {
      const got = await api.listServerBans(server.id)
      if (ask === asked) { bans = got ?? []; error = '' }
    } catch (e) {
      if (ask === asked) error = errorText(e)
    }
  }
  const rev = $derived(app.state.bansRev[server.id] ?? 0)
  $effect(() => {
    void rev
    untrack(() => void load())
  })
  const rows = $derived((bans ?? []).map((b) => ({ id: b.user_id, name: displayName(app.state, server.id, b.user_id), since: formatMemberSince(b.banned_at) })))
  let hovered = $state<string | null>(null)
  let menu = $state<PersonMenuAt | null>(null)
  function openMenu(userId: string, e: MouseEvent | KeyboardEvent, row: HTMLElement) {
    menu = personMenuAt(e, userId, server.id, row, { row: true }) ?? menu
  }
  async function unban(userId: string) {
    error = ''
    try {
      await api.setServerBan(server.id, userId, false)
      bans = (bans ?? []).filter((b) => b.user_id !== userId)
    } catch (e) {
      error = errorText(e)
    }
  }
</script>

<section class="settings-pane" data-server-bans>
  <h2>Bans</h2>
  {#if rows.length}
    <p class="hint">They can't join again until you unban them. Right-click someone to.</p>
    <ul class="list" aria-label="Banned from {server.name}">
      {#each rows as b (b.id)}
        <li><div class="row hover-row" role="presentation" data-banned={b.id} onpointerenter={() => (hovered = b.id)} onpointerleave={() => { if (hovered === b.id) hovered = null }}
          oncontextmenu={(e) => openMenu(b.id, e, e.currentTarget)} onkeydown={(e) => openMenu(b.id, e, e.currentTarget)}>
          <Avatar id={b.id} name={b.name} size={32} interactive serverId={server.id} hovered={() => hovered === b.id} />
          <span class="who">
            <span class="nm"><Name userId={b.id} serverId={server.id} text={b.name} /></span>
            <small class="when">Banned {b.since}</small>
          </span>
        </div></li>
      {/each}
    </ul>
  {:else if bans}
    <p class="empty">Nobody's banned. Right-click a member to ban them.</p>
  {/if}
  {#if error}<p class="err" role="alert">{error}</p>{/if}
</section>
<PersonMenu {menu} onUnban={unban} onClose={() => (menu = null)} />

<style>
  .hint { margin: -12px 0 0; color: var(--text-3); font-size: 12px; }
  .list { margin: 0; padding: 0; list-style: none; display: flex; flex-direction: column; gap: 2px; }
  .row { min-height: 48px; padding: 6px 10px; border-radius: var(--radius-md); display: flex; align-items: center; gap: 12px; }
  .row:hover { background: var(--bg-2); --ring: var(--bg-2); }
  .who { flex: 1; min-width: 0; display: flex; flex-direction: column; font-size: 14px; font-weight: 500; }
  .nm { min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .when { color: var(--text-3); font-size: 12px; font-weight: 400; }
  .empty { margin: 0; color: var(--text-3); font-size: 13px; }
  .err { margin: 0; font-size: 13px; color: var(--danger-text); }
</style>
