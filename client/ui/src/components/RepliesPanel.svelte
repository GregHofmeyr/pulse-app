<script lang="ts">
  // The replies to one quote or clip (spec §3.6): it is pinned at the top, then a normal chat about it.
  // Replies live in `app.state.threads[root]`, never in the channel's list.
  import { tick, untrack } from 'svelte'
  import Composer from './Composer.svelte'
  import Icon from './Icon.svelte'
  import MessageItem from './MessageItem.svelte'
  import QuoteItem from './QuoteItem.svelte'
  import ClipItem from './ClipItem.svelte'
  import { app, ui } from '../lib/store.svelte'
  import { windowFocus } from '../lib/windowFocus.svelte'
  import { addPending, addReplies, closeThread, holdThread, maxIdOf, PAGE, pendingMessage, removePending, threadOf, tracksUnread, type Pending } from '../lib/state'
  import { overlayShown } from '../lib/escape'
  import { groupedWith, keepStable, mentionablesFor, nameOfIn } from '../lib/selectors'
  import { api, errorText } from '../lib/tauri'
  import type { Message } from '../lib/protocol/Message'

  let { root, serverId, onClose, onEditQuote = () => {} }: {
    /** The quote or clip (kept current by the caller). */
    root: Message
    serverId: string
    onClose: () => void
    /** The pinned quote's Edit (the channel's quote composer edits it). */
    onEditQuote?: () => void
  } = $props()

  // One panel per quote: the caller remounts it for another one.
  const rootId = untrack(() => root.id)
  const channelId = untrack(() => root.channel_id)
  const meId = $derived(app.state.me?.id ?? '')
  const replies = $derived(app.state.threads[rootId] ?? [])
  const byId = $derived(new Map(replies.map((m) => [m.id, m])))
  const pending = $derived((app.state.pending[channelId] ?? []).filter((p) => p.thread_root_id === rootId))
  /** Fetched since the last Ready (a reconnect clears it, and the thread is fetched again). */
  const fetched = $derived(rootId in app.state.threadStart)
  const reachedStart = $derived(!!app.state.threadStart[rootId])
  let replyTo = $state<Message | null>(null)
  let error = $state('')
  let panel: HTMLElement
  let list: HTMLDivElement | undefined = $state()
  let composer: Composer | undefined = $state()

  const nameOf = $derived(nameOfIn(() => app.state, serverId))

  // Its replies are on screen while it's open (the message sound skips them).
  $effect(() => {
    ui.openThreadId = rootId
    return () => {
      if (ui.openThreadId === rootId) ui.openThreadId = null
    }
  })

  // The store keeps the quote current while it's open (even once its channel's list lets it go), and
  // the thread is let go when it closes: opening it again refetches. After the flush that closed it: a
  // teardown reads state as it was before that flush, so writing it there would undo the flush's changes.
  $effect(() => {
    const r = root
    untrack(() => (app.state = holdThread(app.state, r)))
  })
  let alive = true
  $effect(() => () => {
    alive = false
    queueMicrotask(() => {
      if (ui.openThreadId !== rootId) app.state = closeThread(app.state, rootId, channelId) // unless it opened again
    })
  })

  // Opening it is for writing: the box takes the focus.
  let focused = false
  $effect(() => {
    if (!composer || focused) return
    focused = true
    composer.focus()
  })

  // The newest page, whenever the thread isn't fetched (first open, or after a reconnect).
  let fetching = false
  $effect(() => {
    if (fetched || fetching) return
    fetching = true
    api.listReplies(rootId).then(
      async (page) => {
        fetching = false
        if (!alive) return // closed meanwhile: the thread was let go, and this page would bring it back
        app.state = addReplies(app.state, rootId, page, page.length < PAGE)
        await tick()
        list?.scrollTo({ top: list.scrollHeight })
      },
      (e) => {
        fetching = false
        error = errorText(e)
      },
    )
  })

  let loadingOlder = false
  async function maybeLoadOlder() {
    if (loadingOlder || !fetched || reachedStart || !list || list.scrollTop > 60 || !replies.length) return
    loadingOlder = true
    const before = list.scrollHeight
    try {
      const page = await api.listReplies(rootId, replies[0].id)
      // A reconnect cleared the thread meanwhile: this page would leave a gap above the refetched one.
      if (!(rootId in app.state.threadStart)) return
      app.state = addReplies(app.state, rootId, page, page.length < PAGE)
      await tick()
      if (list) list.scrollTop = list.scrollHeight - before // keep the view where it was
    } catch (e) {
      error = errorText(e)
    } finally {
      loadingOlder = false
    }
  }

  // Stick to the bottom when replies arrive and we were already near it.
  $effect(() => {
    void replies.length
    void pending.length
    if (!list) return
    const nearBottom = list.scrollHeight - list.scrollTop - list.clientHeight < 120
    if (nearBottom) void tick().then(() => list?.scrollTo({ top: list.scrollHeight }))
  })

  // Seen: up to the newest reply, on opening and as replies arrive, while the window is focused. Only
  // for a member: no one else has a seen point (the server refuses it).
  const thread = $derived(threadOf(root))
  const newest = $derived(maxIdOf(replies.at(-1)?.id, thread?.last_reply_id))
  const seen = $derived(maxIdOf(thread?.replies_seen, app.state.threadSeen[rootId]))
  const member = $derived(tracksUnread(app.state, channelId))
  let asked = ''
  $effect(() => {
    const n = newest
    if (!n || !member || !windowFocus.focused || n <= seen || n <= asked) return
    asked = n
    api.markRepliesSeen(rootId, n).catch(() => {
      if (asked === n) asked = '' // try again with the next reply or focus
    })
  })

  // Everyone in the server can be @mentioned (as in its text channels).
  const mentionables = $derived(mentionablesFor(app.state, channelId))
  const sameNames = keepStable<string>()
  const mentionNames = $derived(sameNames(mentionables.map((p) => p.name)))

  // `replyToId` stays inside the thread: it's never the quote unless the person picked it.
  function post(text: string, replyToId: string | null) {
    const nonce = crypto.randomUUID()
    app.state = addPending(app.state, channelId, { nonce, content: text, reply_to_id: replyToId, thread_root_id: rootId, status: 'pending', created_at: new Date().toISOString() })
    void api.sendReply(channelId, rootId, text, replyToId, nonce)
    void tick().then(() => list?.scrollTo({ top: list.scrollHeight }))
  }
  function send(text: string) {
    const r = replyTo?.id ?? null
    replyTo = null
    post(text, r)
  }
  // A retry keeps its own target and leaves the one being picked for the next reply alone.
  function retry(p: Pending) {
    app.state = removePending(app.state, channelId, p.nonce)
    post(p.content, p.reply_to_id)
  }

  // Escape closes the panel, unless something else takes it: a field outside the panel, or a menu,
  // popover or dialog that's showing (some of them listen on the window after this does).
  const TYPING = 'textarea, select, input, [contenteditable="true"], [contenteditable="plaintext-only"]'
  function key(e: KeyboardEvent) {
    if (e.key !== 'Escape' || e.defaultPrevented) return
    const at = document.activeElement
    if (at && !panel.contains(at) && at.closest(TYPING)) return
    if (overlayShown()) return
    e.preventDefault()
    onClose()
  }
</script>

<svelte:window onkeydown={key} />

<aside class="replies-panel" aria-label="Replies" bind:this={panel}>
  <div class="head">
    <Icon name="reply" size={16} /><span class="title">Replies</span>
    <span class="grow"></span>
    <button class="icon-btn" aria-label="Close replies" data-tip="Close" onclick={onClose}><Icon name="x" size={16} /></button>
  </div>
  <div class="pinned">
    {#if root.clip}
      <ClipItem message={root} {serverId} {meId} {nameOf} compact />
    {:else}
      <QuoteItem message={root} {serverId} {meId} {nameOf} onEdit={onEditQuote} onDelete={() => api.deleteMessage(rootId)} />
    {/if}
  </div>
  <div class="list" bind:this={list} onscroll={() => void maybeLoadOlder()}>
    {#if fetched && reachedStart && replies.length === 0 && pending.length === 0}
      <p class="empty">No replies yet. Say something about it.</p>
    {/if}
    {#each replies as m, i (m.id)}
      <MessageItem message={m} serverId={serverId || null} name={nameOf(m.author_id)} {nameOf} grouped={groupedWith(replies, i)}
        replyTo={m.reply_to_id ? (byId.get(m.reply_to_id) ?? (m.reply_to_id === rootId ? root : null)) : null}
        mine={m.author_id === meId} {mentionNames} {meId}
        onReply={() => (replyTo = m)}
        onEdit={async (c) => { await api.editMessage(m.id, c) }}
        onDelete={() => api.deleteMessage(m.id)} onEditClosed={() => composer?.focus()} />
    {/each}
    {#each pending as p (p.nonce)}
      <MessageItem message={pendingMessage(p, channelId, meId)} serverId={serverId || null} name={nameOf(meId || null)} {nameOf} grouped={false}
        replyTo={p.reply_to_id ? (byId.get(p.reply_to_id) ?? null) : null} mine
        {mentionNames} {meId} pending={p} onReply={() => {}} onEdit={async () => {}} onDelete={async () => {}}
        onRetry={() => retry(p)} onDiscard={() => (app.state = removePending(app.state, channelId, p.nonce))} />
    {/each}
  </div>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  <!-- typing isn't sent: it would show in the channel, where the replies aren't -->
  <Composer bind:this={composer} draftKey="thread:{rootId}" placeholder={root.clip ? 'Reply about this clip' : 'Reply about this quote'} replyingTo={replyTo ? nameOf(replyTo.author_id) : null}
    onCancelReply={() => (replyTo = null)} onSend={(t) => send(t)} onTyping={() => {}} {mentionables} serverId={serverId || null} />
</aside>

<style>
  .replies-panel { position: absolute; top: 0; right: 0; bottom: 0; width: var(--replies-w, 380px); display: flex; flex-direction: column; min-height: 0; background: var(--bg-1); border-left: 1px solid var(--bg-3); z-index: 3; }
  .head { height: 52px; flex-shrink: 0; padding: 0 12px 0 18px; display: flex; align-items: center; gap: 8px; border-bottom: 1px solid var(--bg-3); color: var(--text-3); }
  .title { color: var(--text); font-size: 15px; font-weight: 600; }
  .grow { flex: 1; }
  /* a long quote scrolls on its own, so the replies always get room */
  .pinned { flex-shrink: 0; max-height: 40%; overflow-y: auto; padding: 8px 0 6px; border-bottom: 1px solid var(--bg-3); }
  .list { flex: 1; min-height: 0; overflow-y: auto; padding: 8px 0; display: flex; flex-direction: column; }
  .empty { margin: auto 18px 8px; color: var(--text-3); font-size: 13px; }
  .error { margin: 0 18px 4px; color: var(--danger); font-size: 13px; }
</style>
