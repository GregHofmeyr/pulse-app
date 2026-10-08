<script lang="ts">
  import Icon from './Icon.svelte'
  import Avatar from './Avatar.svelte'
  import { app, openDm } from '../lib/store.svelte'
  import { relativeTime } from '../lib/conversations'
  import { errorText } from '../lib/tauri'

  let { onOpen, onJoinVoice }: {
    onOpen: (channelId: string) => void
    onJoinVoice: (channelId: string, serverId: string) => void
  } = $props()

  let now = $state(Date.now())
  $effect(() => {
    const t = setInterval(() => (now = Date.now()), 30_000)
    return () => clearInterval(t)
  })
  let error = $state('')

  const people = $derived(
    Object.values(app.state.people)
      .filter((p) => p.user.id !== app.state.me?.id)
      .sort((a, b) => Number(b.online) - Number(a.online)
        || (b.last_seen_at ?? '').localeCompare(a.last_seen_at ?? '')
        || a.user.username.localeCompare(b.user.username)),
  )
  const online = $derived(people.filter((p) => p.online))
  const offline = $derived(people.filter((p) => !p.online))

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
          <div class="card" class:unread={unread > 0} class:away={!p.online}>
            <div class="who">
              <Avatar id={p.user.id} name={p.user.username} size={36} online={p.online} />
              <div class="text">
                <strong>{p.user.username}</strong>
                <span class="status">
                  {#if voice}<Icon name="speaker" size={12} /> {voice.label}
                  {:else if unread > 0}{unread} new message{unread === 1 ? '' : 's'}
                  {:else if p.online}online
                  {:else if p.last_seen_at}last seen {relativeTime(p.last_seen_at, now)}
                  {:else}offline{/if}
                </span>
              </div>
            </div>
            <div class="acts">
              <button onclick={() => message(p.user.id)}><Icon name="message" size={14} /> Message</button>
              {#if voice}
                <button onclick={() => onJoinVoice(voice.channelId, voice.serverId)}><Icon name="speaker" size={14} /> Join</button>
              {:else}
                <button disabled title="Calls arrive soon"><Icon name="phone" size={14} /> Call</button>
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
  .card.unread { box-shadow: inset 0 0 0 1px var(--accent); }
  .card.away { opacity: .6; }
  .who { display: flex; align-items: center; gap: 10px; min-width: 0; }
  .text { display: flex; flex-direction: column; min-width: 0; }
  .text strong { font-size: 14px; }
  .status { font-size: 12px; color: var(--text-3); display: flex; align-items: center; gap: 4px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .acts { display: flex; gap: 6px; }
  .acts button { flex: 1; height: 30px; border: 0; border-radius: 8px; background: var(--bg-3); color: var(--text-2); font-size: 12px; font-weight: 600; display: flex; align-items: center; justify-content: center; gap: 5px; }
  .acts button:hover:not(:disabled) { background: var(--bg-4); color: var(--text); }
  .acts button:disabled { opacity: .45; }
  .error { color: #f2616b; font-size: 13px; }
  .empty { padding: 24px 2px; color: var(--text-3); }
</style>
