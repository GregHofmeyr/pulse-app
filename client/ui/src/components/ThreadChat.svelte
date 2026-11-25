<script lang="ts">
  // The chat about one message (a quote's or clip's replies, a build's comments): its replies, the unsent ones,
  // older pages on scroll up, and the box to write in. The caller shows the message itself above it.
  // Replies live in `app.state.threads[root]`, never in the channel's list.
  import { tick, untrack } from 'svelte'
  import { isHidden } from '../lib/blocks'
  import Composer from './Composer.svelte'
  import MessageItem from './MessageItem.svelte'
  import { app, ui } from '../lib/store.svelte'
  import { windowFocus } from '../lib/windowFocus.svelte'
  import { addPending, addReplies, closeThread, holdThread, maxIdOf, PAGE, pendingMessage, removePending, threadOf, tracksUnread, type Pending } from '../lib/state'
  import { pinToBottom } from '../lib/pinBottom'
  import { groupedWith, keepStable, mentionablesFor, nameOfIn, pillNamesFor } from '../lib/selectors'
  import { api, errorText } from '../lib/tauri'
  import type { Message } from '../lib/protocol/Message'
  import type { Gif } from '../lib/protocol/Gif'

  let { root, serverId, placeholder, authorTag = null }: {
    /** The message the chat is about (kept current by the caller). */
    root: Message
    serverId: string
    /** The box's placeholder ("Reply about this quote"). */
    placeholder: string
    /** The user whose replies are tagged "author" (a build's author); none when unset. */
    authorTag?: string | null
  } = $props()

  // One chat per message: the caller remounts it for another one.
  const rootId = untrack(() => root.id)
  const channelId = untrack(() => root.channel_id)
  const meId = $derived(app.state.me?.id ?? '')
  // someone blocked with Hide completely (#474) isn't drawn here either
  const replies = $derived((app.state.threads[rootId] ?? []).filter((m) => !isHidden(app.state, m.author_id)))
  const byId = $derived(new Map(replies.map((m) => [m.id, m])))
  const pending = $derived((app.state.pending[channelId] ?? []).filter((p) => p.thread_root_id === rootId))
  /** Fetched since the last Ready (a reconnect clears it, and the thread is fetched again). */
  const fetched = $derived(rootId in app.state.threadStart)
  const reachedStart = $derived(!!app.state.threadStart[rootId])
  let replyTo = $state<Message | null>(null)
  let error = $state('')
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

  // The store keeps the root current while it's open (even once its channel's list lets it go), and
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
  // ...and stay there as replies grow after they're drawn (#296, as the channel's list does since #292): a link's card
  // unfurling later, images loading, reactions.
  $effect(() => {
    if (!list) return
    const p = pinToBottom(list, { on: () => !loadingOlder })
    return () => p.destroy()
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
  // mentions are written as usernames; pills then show the name that applies (MessageItem)
  const mentionNames = $derived(sameNames(pillNamesFor(app.state, channelId)))

  // `replyToId` stays inside the thread: it's never the root unless the person picked it.
  function post(text: string, replyToId: string | null, gif: Gif | null = null) {
    const nonce = crypto.randomUUID()
    app.state = addPending(app.state, channelId, { nonce, content: text, reply_to_id: replyToId, thread_root_id: rootId, status: 'pending', created_at: new Date().toISOString(), ...(gif ? { gif } : {}) })
    void api.sendReply(channelId, rootId, text, replyToId, nonce, gif)
    void tick().then(() => list?.scrollTo({ top: list.scrollHeight }))
  }
  function send(text: string) {
    const r = replyTo?.id ?? null
    replyTo = null
    post(text, r)
  }
  // a GIF from the picker (#58) goes at once, as a reply to whatever is being replied to in the thread
  function sendGif(gif: Gif, link: string) {
    const r = replyTo?.id ?? null
    replyTo = null
    post(link, r, gif)
  }
  // A retry keeps its own target and leaves the one being picked for the next reply alone.
  function retry(p: Pending) {
    app.state = removePending(app.state, channelId, p.nonce)
    post(p.content, p.reply_to_id, p.gif ?? null)
  }
</script>

<div class="thread-chat">
  <div class="list" bind:this={list} onscroll={() => void maybeLoadOlder()}>
    {#if fetched && reachedStart && replies.length === 0 && pending.length === 0}
      <p class="empty">No replies yet. Say something about it.</p>
    {/if}
    {#each replies as m, i (m.id)}
      <MessageItem message={m} serverId={serverId || null} name={nameOf(m.author_id)} {nameOf} grouped={groupedWith(replies, i)}
        replyTo={m.reply_to_id ? (byId.get(m.reply_to_id) ?? (m.reply_to_id === rootId ? root : null)) : null}
        mine={m.author_id === meId} {mentionNames} {meId} authorTag={!!authorTag && m.author_id === authorTag}
        onReply={() => (replyTo = m)}
        onEdit={async (c) => { await api.editMessage(m.id, c) }}
        onDelete={() => api.deleteMessage(m.id)} onEditClosed={() => composer?.focus()} />
    {/each}
    {#each pending as p (p.nonce)}
      <MessageItem message={pendingMessage(p, channelId, meId)} serverId={serverId || null} name={nameOf(meId || null)} {nameOf} grouped={false}
        replyTo={p.reply_to_id ? (byId.get(p.reply_to_id) ?? null) : null} mine
        {mentionNames} {meId} authorTag={!!authorTag && meId === authorTag} pending={p} onReply={() => {}} onEdit={async () => {}} onDelete={async () => {}}
        onRetry={() => retry(p)} onDiscard={() => (app.state = removePending(app.state, channelId, p.nonce))} />
    {/each}
  </div>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  <!-- typing isn't sent: it would show in the channel, where the replies aren't -->
  <Composer bind:this={composer} draftKey="thread:{rootId}" {placeholder} replyingTo={replyTo ? nameOf(replyTo.author_id) : null}
    onCancelReply={() => (replyTo = null)} onSend={(t) => send(t)} onTyping={() => {}} {mentionables} serverId={serverId || null} onSendGif={sendGif} />
</div>

<style>
  .thread-chat { flex: 1; min-height: 0; display: flex; flex-direction: column; }
  .list { flex: 1; min-height: 0; overflow-y: auto; padding: 8px 0; display: flex; flex-direction: column; }
  .empty { margin: auto 18px 8px; color: var(--text-3); font-size: 13px; }
  .error { margin: 0 18px 4px; color: var(--danger-text); font-size: 13px; }
</style>
