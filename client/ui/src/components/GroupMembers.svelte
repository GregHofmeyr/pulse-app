<script lang="ts">
  import Avatar from './Avatar.svelte'
  import Name from './Name.svelte'
  import ListeningLine from './ListeningLine.svelte'
  import SpotifyIcon from './SpotifyIcon.svelte'
  import { listeningOf } from '../lib/activity'
  import { app, clock } from '../lib/store.svelte'
  import { byPresence, presenceOf } from '../lib/presence'
  import { api, errorText } from '../lib/tauri'
  import { displayName } from '../lib/selectors'
  import { isMenuKey, rowMenuAt } from '../lib/participantMenu'
  import MemberMenu from './MemberMenu.svelte'

  let { channelId, onMessage }: { channelId: string; onMessage?: (userId: string) => void } = $props()
  // by the name you see them by (#47)
  const members = $derived(
    (app.state.dmMembers[channelId] ?? [])
      .map((id) => app.state.people[id])
      .filter((p) => !!p)
      .map((p) => ({ ...p, name: displayName(app.state, null, p.user.id) }))
      .sort((a, b) => a.name.localeCompare(b.name)),
  )
  // Online (DND included) above Offline (#170), as on the server member list (MemberList.svelte)
  const sections = $derived.by(() => {
    const { online, offline } = byPresence(app.state, members, (p) => p.user.id, clock.nowIso)
    return [{ key: 'online', label: 'Online', list: online }, { key: 'offline', label: 'Offline', list: offline }].filter((g) => g.list.length)
  })
  let error = $state('')
  // the row under the pointer: its avatar animates
  let hovered = $state<string | null>(null)

  // A row's menu (#135): the server member list's person menu (MemberMenu) with no server, so View profile, Message,
  // Change nickname (your private one) and the other person-to-person items, then Remove from group (#335: was a
  // hover X on the row; anyone but you, as before). Right-click, or ContextMenu / Shift+F10 on the row's name or avatar.
  let menu = $state<{ userId: string; x: number; y: number; opener: HTMLElement } | null>(null)
  function openMenu(userId: string, e: MouseEvent | KeyboardEvent, row: HTMLElement) {
    e.preventDefault()
    menu = { userId, ...rowMenuAt(e, row) }
  }

  async function remove(id: string) {
    error = ''
    try {
      await api.removeMember(channelId, id)
    } catch (e) {
      error = errorText(e)
    }
  }
</script>

<aside class="members" aria-label="Group members">
  {#each sections as g (g.key)}
  <div class="label">{g.label} — {g.list.length}</div>
  {#each g.list as p (p.user.id)}
    {@const listening = listeningOf(app.state, p.user.id)}
    <div class="row hover-row" class:off={g.key === 'offline'} data-section={g.key} role="presentation" onpointerenter={() => (hovered = p.user.id)} onpointerleave={() => { if (hovered === p.user.id) hovered = null }}
      oncontextmenu={(e) => openMenu(p.user.id, e, e.currentTarget)} onkeydown={(e) => { if (isMenuKey(e)) openMenu(p.user.id, e, e.currentTarget) }}>
      <Avatar id={p.user.id} name={p.name} size={32} presence={presenceOf(app.state, p.user.id, clock.nowIso)} interactive serverId={null} hovered={() => hovered === p.user.id} />
      <!-- what they're listening to (#275) on the line under the name; Spotify's icon at the row's end -->
      <span class="who"><span class="nm"><Name userId={p.user.id} serverId={null} text={p.name} surface="side" />{#if p.user.id === app.state.me?.id}<small> (you)</small>{/if}</span>{#if listening}<ListeningLine {listening} icon={false} />{/if}</span>
      {#if listening}<span class="sp"><SpotifyIcon /></span>{/if}
    </div>
  {/each}
  {/each}
  {#if error}<p class="err" role="alert">{error}</p>{/if}
</aside>
{#if menu}
  {#key menu}<MemberMenu serverId={null} userId={menu.userId} x={menu.x} y={menu.y} opener={menu.opener} {onMessage} onRemove={remove} onClose={() => (menu = null)} />{/key}
{/if}

<style>
  .members { width: var(--right-w, 220px); flex-shrink: 0; background: var(--bg-1); border-radius: var(--radius); padding: 8px; overflow-y: auto; }
  .label { padding: 14px 10px 6px; font-size: 11px; font-weight: 600; letter-spacing: .06em; text-transform: uppercase; color: var(--text-3); }
  .row { height: 44px; padding: 0 10px; border-radius: var(--radius-md); display: flex; align-items: center; gap: 10px; }
  .row:hover { background: var(--bg-2); --ring: var(--bg-2); }
  /* offline (#170): dimmed slightly, as on Discord; back to full while hovered */
  .row.off { opacity: .55; }
  .row.off:hover { opacity: 1; }
  /* the status dot (#169): small, cut out of the avatar in the row's own colour (--ring), as on Discord */
  .row :global(.pres) { width: 12px; height: 12px; min-width: 0; min-height: 0; right: -1px; bottom: -1px; }
  .who { flex: 1; min-width: 0; font-size: 14px; font-weight: 500; display: flex; flex-direction: column; }
  .nm { min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .sp { flex-shrink: 0; color: var(--text-3); }
  .who small { color: var(--text-3); font-weight: 400; }
  .err { margin: 8px 10px; font-size: 13px; color: var(--danger-text); }
</style>
