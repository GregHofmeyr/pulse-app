<script lang="ts">
  import { tick, untrack } from 'svelte'
  import Icon from './Icon.svelte'
  import MessageItem from './MessageItem.svelte'
  import Composer from './Composer.svelte'
  import Avatar from './Avatar.svelte'
  import ConversationMenu from './ConversationMenu.svelte'
  import { app, clock, ui } from '../lib/store.svelte'
  import { conversationName, firstUnreadIndex, isMuted, reanchor, shouldMarkRead } from '../lib/conversations'
  import { addHistory, addPending, markHistory } from '../lib/state'
  import { displayName, typingNames } from '../lib/selectors'
  import { api, errorText } from '../lib/tauri'
  import type { Message } from '../lib/protocol/Message'

  let { channelId, serverId, onAddPeople = () => {}, onLeft = () => {} }: {
    channelId: string
    serverId: string | null
    /** DMs/groups: open the people picker. */
    onAddPeople?: () => void
    /** After leaving/closing this conversation. */
    onLeft?: () => void
  } = $props()

  const channel = $derived(app.state.channels[channelId])
  const messages = $derived(app.state.messages[channelId] ?? [])
  const pending = $derived(app.state.pending[channelId] ?? [])
  const byId = $derived(new Map(messages.map((m) => [m.id, m])))
  let now = $state(Date.now())
  const typers = $derived(typingNames(app.state, serverId, channelId, now))
  let replyTo = $state<Message | null>(null)
  let error = $state('')
  let list: HTMLDivElement
  let loadingOlder = false
  let fetching: string | null = null
  const history = $derived(app.state.history[channelId])
  const reachedStart = $derived(!!history?.start)

  // fade typing indicators on time, not just on events
  $effect(() => {
    const t = setInterval(() => (now = Date.now()), 1000)
    return () => clearInterval(t)
  })

  $effect(() => {
    void channelId
    replyTo = null
  })

  // Fetch the latest page whenever this channel isn't marked loaded (first view, or after a
  // reconnect cleared the markers). Marked only on success, so an offline failure retries later.
  $effect(() => {
    const c = channelId
    if (history?.loaded || fetching === c) return
    fetching = c
    api.listMessages(c).then(
      async (page) => {
        app.state = markHistory(addHistory(app.state, c, page), c, page.length < 50)
        fetching = null
        await tick()
        list?.scrollTo({ top: list.scrollHeight })
      },
      (e) => {
        fetching = null
        error = errorText(e)
      },
    )
  })

  // stick to the bottom when new messages arrive and we were already near it
  $effect(() => {
    void messages.length
    void pending.length
    if (!list) return
    const nearBottom = list.scrollHeight - list.scrollTop - list.clientHeight < 120
    if (nearBottom) tick().then(() => list.scrollTo({ top: list.scrollHeight }))
  })

  async function maybeLoadOlder() {
    if (loadingOlder || reachedStart || list.scrollTop > 60 || !messages.length) return
    loadingOlder = true
    const before = list.scrollHeight
    try {
      const page = await api.listMessages(channelId, messages[0].id)
      app.state = markHistory(addHistory(app.state, channelId, page), channelId, page.length < 50)
      await tick()
      list.scrollTop = list.scrollHeight - before // keep the view where it was
    } catch (e) {
      error = errorText(e)
    } finally {
      loadingOlder = false
    }
  }

  // Who can be @mentioned here: a DM/group's members, or everyone for server channels.
  const mentionables = $derived.by(() => {
    const ids = channel?.server_id === null ? (app.state.dmMembers[channelId] ?? []) : Object.keys(app.state.people)
    return ids
      .filter((id) => id !== app.state.me?.id)
      .map((id) => ({ id, name: app.state.people[id]?.user.username ?? '' }))
      .filter((p) => p.name)
  })
  const mentionNames = $derived(mentionables.map((p) => p.name))

  // --- DM/group header ---
  const isPrivate = $derived(channel?.server_id === null)
  const title = $derived(isPrivate ? conversationName(app.state, channelId) : (channel?.name ?? ''))
  const others = $derived((app.state.dmMembers[channelId] ?? []).filter((u) => u !== app.state.me?.id))
  const muted = $derived(channel ? isMuted(app.state, channel, clock.nowIso) : false)
  let menu = $state<null | 'mute' | 'manage'>(null)
  let renaming = $state(false)
  let renameValue = $state('')
  function startRename() {
    renameValue = channel?.name ?? ''
    renaming = true
  }
  async function saveRename() {
    if (!renaming) return
    renaming = false
    const v = renameValue.trim()
    if (v === (channel?.name ?? '')) return
    try {
      await api.renameChannel(channelId, v || null)
    } catch (e) {
      error = errorText(e)
    }
  }

  // --- unread: the NEW line starts at the read point you opened with, then moves to the newest
  // message each time you stop watching (unfocus / scroll up), so every away stretch gets one ---
  let anchor = $state(untrack(() => app.state.reads[channelId]?.last_read_message_id ?? null))
  let anchored = $state(untrack(() => channelId in app.state.reads)) // brand-new chats start without a line
  const newIndex = $derived(anchored ? firstUnreadIndex(messages, anchor, app.state.me?.id ?? '') : -1)
  let atBottom = $state(true)
  let wasWatching = false
  $effect(() => {
    const now = ui.focused && atBottom
    const newest = untrack(() => messages.at(-1)?.id ?? null)
    const next = reanchor(untrack(() => anchor), { was: wasWatching, now, newest })
    if (next !== untrack(() => anchor)) {
      anchor = next
      anchored = true
    }
    wasWatching = now
  })
  const unreadBelow = $derived(atBottom ? 0 : (app.state.reads[channelId]?.unread ?? 0))
  let markTimer: ReturnType<typeof setTimeout> | null = null
  $effect(() => {
    const newest = messages.at(-1)?.id
    const read = app.state.reads[channelId]?.last_read_message_id ?? null
    const want = shouldMarkRead({ open: true, focused: ui.focused, atBottom })
    if (!newest || !want || (read !== null && newest <= read)) return
    if (markTimer) clearTimeout(markTimer)
    markTimer = setTimeout(() => void api.markRead(channelId, newest).catch(() => {}), 300)
    return () => {
      if (markTimer) clearTimeout(markTimer)
    }
  })
  function onScroll() {
    atBottom = list.scrollHeight - list.scrollTop - list.clientHeight < 40
    void maybeLoadOlder()
  }
  function jump() {
    list.scrollTo({ top: list.scrollHeight, behavior: 'smooth' })
  }

  const nameOf = (userId: string | null) => (userId ? displayName(app.state, serverId, userId) : 'Pulse')

  function send(text: string) {
    const nonce = crypto.randomUUID()
    app.state = addPending(app.state, channelId, { nonce, content: text, reply_to_id: replyTo?.id ?? null, status: 'pending' })
    const r = replyTo?.id ?? null
    replyTo = null
    void api.sendMessage(channelId, text, r, nonce)
    // Sending from higher up takes you to the latest (your message lands there).
    void tick().then(() => list?.scrollTo({ top: list.scrollHeight }))
  }

  function retry(nonce: string) {
    const p = pending.find((x) => x.nonce === nonce)
    if (!p) return
    app.state = { ...app.state, pending: { ...app.state.pending, [channelId]: pending.filter((x) => x.nonce !== nonce) } }
    replyTo = p.reply_to_id ? (byId.get(p.reply_to_id) ?? null) : null
    send(p.content)
  }

  const grouped = (i: number) => {
    const m = messages[i]
    const prev = messages[i - 1]
    return !!prev && prev.author_id === m.author_id && prev.kind === 'normal' && !m.reply_to_id
      && new Date(m.created_at).getTime() - new Date(prev.created_at).getTime() < 5 * 60_000
  }
</script>

<div class="chan">
  <div class="head">
    {#if isPrivate && channel?.kind === 'dm' && others[0]}
      <Avatar id={others[0]} name={title} size={26} online={app.state.people[others[0]]?.online ?? false} />
      <span class="title">{title}</span>
    {:else if isPrivate}
      <span class="pile">
        {#each others.slice(0, 3) as u (u)}<Avatar id={u} name={app.state.people[u]?.user.username ?? '?'} size={22} />{/each}
      </span>
      {#if renaming}
        <!-- svelte-ignore a11y_autofocus -->
        <input class="rename" maxlength="64" placeholder={conversationName({ ...app.state, channels: { ...app.state.channels, [channelId]: { ...channel!, name: null } } }, channelId)}
          bind:value={renameValue} autofocus onblur={saveRename}
          onkeydown={(e) => { if (e.key === 'Enter') void saveRename(); if (e.key === 'Escape') renaming = false }} />
      {:else}
        <button class="title editable" data-tip="Rename group" onclick={startRename}>{title}</button>
      {/if}
    {:else}
      <Icon name="hash" /> <span class="title">{title}</span>
    {/if}
    <span class="grow"></span>
    {#if isPrivate}
      <button class="hbtn" aria-label="Add people" data-tip="Add people" onclick={onAddPeople}><Icon name="userPlus" size={17} /></button>
    {/if}
    <button class="hbtn" class:on={muted} aria-label={muted ? 'Unmute' : 'Mute'} data-tip={muted ? 'Muted — click to unmute' : 'Mute'} data-menu-toggle
      aria-haspopup={muted ? undefined : 'menu'} aria-expanded={menu === 'mute'}
      onclick={() => (muted ? void api.clearMute('channel', channelId) : (menu = menu === 'mute' ? null : 'mute'))}><Icon name={muted ? 'bellOff' : 'bell'} size={17} /></button>
    {#if isPrivate}
      <button class="hbtn" aria-label="More" aria-haspopup="menu" aria-expanded={menu === 'manage'} data-tip="More" data-menu-toggle
        onclick={() => (menu = menu === 'manage' ? null : 'manage')}><Icon name="dots" size={17} /></button>
    {/if}
    {#if menu && channel}
      <ConversationMenu {channel} {muted} show={menu} onClose={() => (menu = null)} onRename={startRename} {onLeft} />
    {/if}
  </div>
  <div class="list" bind:this={list} onscroll={onScroll}>
    {#if reachedStart}<div class="start">
      {#if channel?.kind === 'dm'}This is the start of your conversation with {title}.
      {:else if channel?.kind === 'group'}This is the start of {title}.
      {:else}This is the start of #{title}.{/if}
    </div>{/if}
    {#each messages as m, i (m.id)}
      {#if i === newIndex}<div class="newline" role="separator" aria-label="New messages">NEW</div>{/if}
      <MessageItem message={m} name={nameOf(m.author_id)} {nameOf} grouped={grouped(i)}
        replyTo={m.reply_to_id ? (byId.get(m.reply_to_id) ?? null) : null}
        mine={m.author_id === app.state.me?.id}
        {mentionNames} meId={app.state.me?.id ?? ''}
        onReply={() => (replyTo = m)}
        onEdit={async (c) => { try { await api.editMessage(m.id, c) } catch (e) { error = errorText(e) } }}
        onDelete={async () => { try { await api.deleteMessage(m.id) } catch (e) { error = errorText(e) } }} />
    {/each}
    {#each pending as p (p.nonce)}
      <div class="pending" class:failed={p.status === 'failed'}>
        <span>{p.content}</span>
        {#if p.status === 'failed'}<button onclick={() => retry(p.nonce)}>Failed to send · Retry</button>
        {:else}<small>sending…</small>{/if}
      </div>
    {/each}
  </div>
  {#if unreadBelow > 0}<button class="jump" onclick={jump}>{unreadBelow} new · Jump</button>{/if}
  <div class="typing" aria-live="polite">
    {#if typers.length}<strong>{typers.join(', ')}</strong> {typers.length > 1 ? 'are' : 'is'} typing…{/if}
  </div>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  <Composer draftKey={channelId} placeholder={isPrivate ? `Message ${title}` : `Message #${title}`} replyingTo={replyTo ? nameOf(replyTo.author_id) : null}
    onCancelReply={() => (replyTo = null)} onSend={send} onTyping={() => void api.sendTyping(channelId)} {mentionables} />
</div>

<style>
  .chan { flex: 1; display: flex; flex-direction: column; min-height: 0; }
  .head { height: 52px; flex-shrink: 0; padding: 0 18px; display: flex; align-items: center; gap: 10px; border-bottom: 1px solid var(--bg-3); color: var(--text-3); }
  .head { position: relative; }
  .title { color: var(--text); font-size: 15px; font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .editable { border: 0; background: none; padding: 2px 4px; margin-left: -4px; border-radius: 6px; font: inherit; font-size: 15px; font-weight: 600; color: var(--text); }
  .editable:hover { background: var(--bg-3); }
  .rename { height: 30px; min-width: 220px; padding: 0 8px; border: 1px solid var(--accent); border-radius: 8px; background: var(--bg-2); color: var(--text); font: inherit; font-weight: 600; outline: none; }
  .pile { display: flex; }
  .pile :global(.av) { box-shadow: 0 0 0 2px var(--bg-1); }
  .pile :global(.av + .av) { margin-left: -7px; }
  .grow { flex: 1; }
  .hbtn { width: 32px; height: 32px; border: 0; border-radius: 9px; background: transparent; color: var(--text-3); display: grid; place-items: center; }
  .hbtn:hover, .hbtn.on { background: var(--bg-3); color: var(--text); }
  .newline { display: flex; align-items: center; gap: 8px; margin: 6px 18px; color: var(--danger); font-size: 11px; font-weight: 700; letter-spacing: .04em; }
  .newline::before, .newline::after { content: ''; flex: 1; height: 1px; background: var(--danger); opacity: .6; }
  .jump { align-self: center; margin-top: -40px; position: relative; z-index: 2; padding: 6px 12px; border: 0; border-radius: 16px; background: var(--accent); color: var(--on-accent); font-size: 12px; font-weight: 700; box-shadow: 0 4px 14px rgba(0, 0, 0, .35); }
  .list { flex: 1; min-height: 0; overflow-y: auto; padding: 12px 0 8px; display: flex; flex-direction: column; }
  .start { padding: 16px 18px; color: var(--text-3); font-size: 13px; }
  .pending { padding: 4px 18px 4px 72px; color: var(--text-3); display: flex; gap: 10px; align-items: baseline; }
  .pending.failed { color: #f2616b; }
  .pending button { border: 0; background: none; color: #f2616b; text-decoration: underline; font-size: 12px; padding: 0; }
  .pending small { font-size: 11px; }
  .typing { height: 22px; padding: 0 18px; font-size: 12px; color: var(--text-3); }
  .typing strong { color: var(--text); }
  .error { margin: 0 18px 4px; color: #f2616b; font-size: 13px; }
</style>
