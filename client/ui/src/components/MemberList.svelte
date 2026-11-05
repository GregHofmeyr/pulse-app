<script lang="ts">
  import { app, clock } from '../lib/store.svelte'
  import { presenceOf } from '../lib/presence'
  import Avatar from './Avatar.svelte'
  import Icon from './Icon.svelte'
  import Name from './Name.svelte'
  import MemberMenu from './MemberMenu.svelte'
  import { baseName, displayName } from '../lib/selectors'
  import { menuPoint } from '../lib/participantMenu'

  let { serverId, onMessage }: { serverId: string; onMessage?: (userId: string) => void } = $props()
  const members = $derived(app.state.members[serverId] ?? [])
  // the row under the pointer: its avatar animates
  let hovered = $state<string | null>(null)

  // A row's menu (#113): right-click, or ContextMenu / Shift+F10 on its name or avatar. Focus goes
  // back to what had it (from the keyboard), else to the row's name.
  let menu = $state<{ userId: string; x: number; y: number; opener: HTMLElement } | null>(null)
  function openMenu(userId: string, e: MouseEvent | KeyboardEvent, row: HTMLElement) {
    const focused = document.activeElement instanceof HTMLElement && row.contains(document.activeElement) ? document.activeElement : null
    const opener = (e instanceof KeyboardEvent && focused) || row.querySelector<HTMLElement>('.name') || row
    menu = { userId, ...menuPoint(e, opener), opener }
  }
  function rowKey(userId: string, e: KeyboardEvent) {
    if (e.key === 'ContextMenu' || (e.key === 'F10' && e.shiftKey)) {
      e.preventDefault()
      openMenu(userId, e, e.currentTarget as HTMLElement)
    }
  }
</script>

<aside class="members" aria-label="Members">
  <div class="label">MEMBERS — {members.length}</div>
  {#each members as m (m.user.id)}
    {@const name = displayName(app.state, serverId, m.user.id)}
    {@const base = baseName(m.user)}
    <!-- the context menu is a pointer shortcut; the keyboard route is ContextMenu / Shift+F10 on the row's buttons -->
    <div class="row" role="presentation" onpointerenter={() => (hovered = m.user.id)} onpointerleave={() => { if (hovered === m.user.id) hovered = null }}
      oncontextmenu={(e) => { e.preventDefault(); openMenu(m.user.id, e, e.currentTarget) }} onkeydown={(e) => rowKey(m.user.id, e)}>
      <Avatar id={m.user.id} {name} size={32} presence={presenceOf(app.state, m.user.id, clock.nowIso)} interactive {serverId} hovered={() => hovered === m.user.id} />
      <span class="who"><Name userId={m.user.id} {serverId} text={name} surface="side" />{#if name !== base}<small>{base}</small>{/if}</span>
      {#if onMessage && m.user.id !== app.state.me?.id}
        <button class="msg" aria-label="Message {name}" data-tip="Message" onclick={() => onMessage(m.user.id)}><Icon name="message" size={15} /></button>
      {/if}
    </div>
  {/each}
</aside>
{#if menu}
  {#key menu}<MemberMenu {serverId} userId={menu.userId} x={menu.x} y={menu.y} opener={menu.opener} {onMessage} onClose={() => (menu = null)} />{/key}
{/if}

<style>
  .members { width: var(--right-w, 240px); flex-shrink: 0; background: var(--bg-1); border-radius: var(--radius); padding: 8px; overflow-y: auto; }
  .label { padding: 14px 10px 6px; font-size: 11px; font-weight: 600; letter-spacing: .06em; color: var(--text-3); }
  .row { height: 44px; padding: 0 10px; border-radius: 10px; display: flex; align-items: center; gap: 10px; }
  /* 15 px: a styled name font (script, pixel) blurs at smaller sizes */
  .who { display: flex; flex-direction: column; font-size: 15px; font-weight: 500; min-width: 0; }
  .who small { font-size: 11px; color: var(--text-3); font-weight: 400; }
  .msg { margin-left: auto; width: 28px; height: 28px; border: 0; border-radius: 8px; background: var(--bg-3); color: var(--text-2); display: grid; place-items: center; opacity: 0; }
  .msg:hover { background: var(--bg-4); color: var(--text); }
  .row:hover .msg, .msg:focus-visible { opacity: 1; }
  .row:hover { background: var(--bg-2); --ring: var(--bg-2); }
  /* the status dot (#169): small, cut out of the avatar in the row's own colour (--ring), as on Discord */
  .row :global(.pres) { width: 12px; height: 12px; min-width: 0; min-height: 0; right: -1px; bottom: -1px; }
</style>
