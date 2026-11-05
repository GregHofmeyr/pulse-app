<script lang="ts">
  import Icon from './Icon.svelte'
  import Avatar from './Avatar.svelte'
  import Name from './Name.svelte'
  import { app, clock, openDm } from '../lib/store.svelte'
  import { isOn, presenceOf } from '../lib/presence'
  import { relativeTime } from '../lib/conversations'
  import { errorText } from '../lib/tauri'
  import { displayName } from '../lib/selectors'

  let { onOpen, onJoinVoice }: {
    onOpen: (channelId: string) => void
    onJoinVoice: (channelId: string, serverId: string) => void
  } = $props()

  let error = $state('')
  // the card under the pointer: its avatar animates
  let hovered = $state<string | null>(null)

  // by the name you see them by (#47); on (online or DND) first, then the most recently seen (#45: presenceOf)
  const people = $derived(
    Object.values(app.state.people)
      .filter((p) => p.user.id !== app.state.me?.id)
      .map((p) => ({ ...p, name: displayName(app.state, null, p.user.id), presence: presenceOf(app.state, p.user.id, clock.nowIso) }))
      .sort((a, b) => Number(isOn(b.presence)) - Number(isOn(a.presence))
        || (b.presence.lastSeen ?? '').localeCompare(a.presence.lastSeen ?? '')
        || a.name.localeCompare(b.name)),
  )
  const online = $derived(people.filter((p) => isOn(p.presence)))
  const offline = $derived(people.filter((p) => !isOn(p.presence)))

  /** A server voice room this person is in (DM/group calls never show here). */
  function voiceOf(userId: string): { channelId: string; serverId: string; label: string } | null {
    for (const [channelId, members] of Object.entries(app.state.voice)) {
      const ch = app.state.channels[channelId]
      if (!ch?.server_id || !members.some((m) => m.user_id === userId)) continue
      const server = app.state.servers.find((s) => s.id === ch.server_id)
      return { channelId, serverId: ch.server_id, label: `in ${server?.name ?? 'a server'} · ${ch.name ?? 'voice'}` }
    }
    return null
  }

  /** Unread in your 1:1 DM with this person. */
  function dmUnread(userId: string): number {
    for (const [id, members] of Object.entries(app.state.dmMembers)) {
      if (app.state.channels[id]?.kind === 'dm' && members.includes(userId)) return app.state.reads[id]?.unread ?? 0
    }
    return 0
  }

  async function message(userId: string) {
    error = ''
    try {
      onOpen(await openDm(userId))
    } catch (e) {
      error = errorText(e)
    }
  }
</script>

<div class="board">
  <div class="head">Your people</div>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  {#each [{ label: 'ONLINE', list: online }, { label: 'OFFLINE', list: offline }] as section (section.label)}
    {#if section.list.length}
      <div class="lbl">{section.label} — {section.list.length}</div>
      <div class="grid">
        {#each section.list as p (p.user.id)}
          {@const voice = voiceOf(p.user.id)}
          {@const unread = dmUnread(p.user.id)}
          <div class="card" class:unread={unread > 0} class:away={!isOn(p.presence)} role="presentation"
            onpointerenter={() => (hovered = p.user.id)} onpointerleave={() => { if (hovered === p.user.id) hovered = null }}>
            <div class="who">
              <Avatar id={p.user.id} name={p.name} size={36} presence={p.presence} interactive serverId={null} hovered={() => hovered === p.user.id} />
              <div class="text">
                <strong><Name userId={p.user.id} serverId={null} text={p.name} /></strong>
                <span class="status">
                  {#if voice}<Icon name="speaker" size={12} /> {voice.label}
                  {:else if unread > 0}{unread} new message{unread === 1 ? '' : 's'}
                  {:else if isOn(p.presence)}{p.presence.label.toLowerCase()}
                  {:else if p.presence.lastSeen}last seen {relativeTime(p.presence.lastSeen, Date.parse(clock.nowIso))}
                  {:else}offline{/if}
                </span>
              </div>
            </div>
            <div class="acts">
              <button class="btn btn-quiet" onclick={() => message(p.user.id)}><Icon name="message" size={14} /> Message</button>
              {#if voice}
                <button class="btn btn-quiet" onclick={() => onJoinVoice(voice.channelId, voice.serverId)}><Icon name="speaker" size={14} /> Join</button>
              {:else}
                <button class="btn btn-quiet" disabled title="Calls arrive soon"><Icon name="phone" size={14} /> Call</button>
              {/if}
            </div>
          </div>
        {/each}
      </div>
    {/if}
  {:else}
    <p class="empty">Nobody else is here yet. Invite your friends!</p>
  {/each}
</div>

<style>
  .board { flex: 1; min-height: 0; overflow-y: auto; padding: 0 18px 18px; }
  .head { height: 52px; display: flex; align-items: center; font-size: 15px; font-weight: 600; border-bottom: 1px solid var(--bg-3); margin: 0 -18px 4px; padding: 0 18px; }
  .lbl { padding: 14px 2px 8px; font-size: 11px; font-weight: 600; letter-spacing: .06em; color: var(--text-3); }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(220px, 1fr)); gap: 10px; }
  .card { background: var(--bg-2); border-radius: 12px; padding: 12px; display: flex; flex-direction: column; gap: 10px; }
  /* unread as the channel and DM lists show it (bright, bold text), not an accent ring that reads as keyboard focus */
  .card.unread .status { color: var(--text); font-weight: 600; }
  .card.away { opacity: .6; }
  .who { display: flex; align-items: center; gap: 10px; min-width: 0; }
  .text { display: flex; flex-direction: column; min-width: 0; }
  .text strong { font-size: 14px; }
  .status { font-size: 12px; color: var(--text-3); display: flex; align-items: center; gap: 4px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .acts { display: flex; gap: 6px; }
  .acts .btn { flex: 1; gap: 5px; }
  .error { color: var(--danger-text); font-size: 13px; }
  .empty { padding: 24px 2px; color: var(--text-3); }
</style>
