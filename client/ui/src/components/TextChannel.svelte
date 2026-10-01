<script lang="ts" module>
  // Channels whose history has been fetched, and those where we've reached the very first message.
  const loaded = new Set<string>()
  const reachedStart = new Set<string>()
</script>

<script lang="ts">
  import { tick } from 'svelte'
  import Icon from './Icon.svelte'
  import MessageItem from './MessageItem.svelte'
  import Composer from './Composer.svelte'
  import { app } from '../lib/store.svelte'
  import { addHistory, addPending } from '../lib/state'
  import { displayName, typingNames } from '../lib/selectors'
  import { api, errorText } from '../lib/tauri'
  import type { Message } from '../lib/protocol/Message'

  let { channelId, serverId }: { channelId: string; serverId: string | null } = $props()

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

  // fade typing indicators on time, not just on events
  $effect(() => {
    const t = setInterval(() => (now = Date.now()), 1000)
    return () => clearInterval(t)
  })

  $effect(() => {
    const c = channelId
    replyTo = null
    if (loaded.has(c)) return
    loaded.add(c)
    api.listMessages(c).then(async (page) => {
      if (page.length < 50) reachedStart.add(c)
      app.state = addHistory(app.state, c, page)
      await tick()
      list?.scrollTo({ top: list.scrollHeight })
    }, (e) => (error = errorText(e)))
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
    if (loadingOlder || reachedStart.has(channelId) || list.scrollTop > 60 || !messages.length) return
    loadingOlder = true
    const before = list.scrollHeight
    try {
      const page = await api.listMessages(channelId, messages[0].id)
      if (page.length < 50) reachedStart.add(channelId)
      app.state = addHistory(app.state, channelId, page)
      await tick()
      list.scrollTop = list.scrollHeight - before // keep the view where it was
    } catch (e) {
      error = errorText(e)
    } finally {
      loadingOlder = false
    }
  }

  const nameOf = (userId: string | null) => (userId ? displayName(app.state, serverId, userId) : 'Pulse')

  function send(text: string) {
    const nonce = crypto.randomUUID()
    app.state = addPending(app.state, channelId, { nonce, content: text, reply_to_id: replyTo?.id ?? null, status: 'pending' })
    const r = replyTo?.id ?? null
    replyTo = null
    void api.sendMessage(channelId, text, r, nonce)
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
  <div class="head"><Icon name="hash" /> <span>{channel?.name}</span></div>
  <div class="list" bind:this={list} onscroll={maybeLoadOlder}>
    {#if reachedStart.has(channelId)}<div class="start">This is the start of #{channel?.name}.</div>{/if}
    {#each messages as m, i (m.id)}
      <MessageItem message={m} name={nameOf(m.author_id)} {nameOf} grouped={grouped(i)}
        replyTo={m.reply_to_id ? (byId.get(m.reply_to_id) ?? null) : null}
        mine={m.author_id === app.state.me?.id}
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
  <div class="typing" aria-live="polite">
    {#if typers.length}<strong>{typers.join(', ')}</strong> {typers.length > 1 ? 'are' : 'is'} typing…{/if}
  </div>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  <Composer placeholder="Message #{channel?.name ?? ''}" replyingTo={replyTo ? nameOf(replyTo.author_id) : null}
    onCancelReply={() => (replyTo = null)} onSend={send} onTyping={() => void api.sendTyping(channelId)} />
</div>

<style>
  .chan { flex: 1; display: flex; flex-direction: column; min-height: 0; }
  .head { height: 52px; flex-shrink: 0; padding: 0 18px; display: flex; align-items: center; gap: 10px; border-bottom: 1px solid var(--bg-3); color: var(--text-3); }
  .head span { color: var(--text); font-size: 15px; font-weight: 600; }
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
