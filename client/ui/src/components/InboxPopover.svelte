<script lang="ts">
  // The inbox (#306): a popover under the top bar's inbox button, styled as the pins popover is. Tabs All (mentions and
  // replies) and Mentions, remembered per device; newest first, the next page loading as you scroll near the end.
  // Each row: who, where, when, a one-line preview with your mention picked out, and for a reply the line it answered.
  // Unread rows stand out. A click shows the message and reads it; Mark all read reads everything shown.
  import { clickOutside } from '../lib/actions'
  import { relativeTime } from '../lib/conversations'
  import { badgeText, highlightParts, oneLine, whereLabel, type InboxTab } from '../lib/inbox'
  import { inbox, loadMore, markAllRead, openItem, setTab } from '../lib/inbox.svelte'
  import { mentionLookup, showMentions } from '../lib/mentionStyles'
  import { shownText } from '../lib/poll'
  import { displayName } from '../lib/selectors'
  import { app, clock } from '../lib/store.svelte'
  import type { InboxItem } from '../lib/protocol/InboxItem'
  import Avatar from './Avatar.svelte'
  import Loading from './Loading.svelte'
  import Segmented from './Segmented.svelte'
  import MemberMenu from './MemberMenu.svelte'
  import { isMenuKey, menuPoint } from '../lib/participantMenu'
  import { messagePerson } from '../lib/messagePerson'

  let { onClose }: { onClose: (e?: Event) => void } = $props()

  const TABS: { value: InboxTab; label: string }[] = [
    { value: 'all', label: 'All' },
    { value: 'mentions', label: 'Mentions' },
  ]
  const list = $derived(inbox.lists[inbox.tab])
  const lookup = $derived(mentionLookup(app.state))
  const meId = $derived(app.state.me?.id ?? null)
  const nowMs = $derived(Date.parse(clock.nowIso))
  /** The name that applies where the message is (a server nickname in its server). */
  const nameIn = (item: InboxItem, userId: string | null) =>
    userId ? displayName(app.state, app.state.channels[item.message.channel_id]?.server_id ?? null, userId) : 'Someone'
  const parts = (item: InboxItem) => highlightParts(oneLine(shownText(item.message)), lookup, (id) => nameIn(item, id), meId)
  const snippet = (item: InboxItem) => {
    const r = item.replied_to
    if (!r) return null
    const text = r.deleted ? 'a deleted message' : oneLine(showMentions(r.content, lookup, (id) => nameIn(item, id)))
    return { who: r.author_id === meId ? 'you' : nameIn(item, r.author_id), text, deleted: r.deleted }
  }

  // A row's person menu (#135): its author's, in the server it was said in. Right-click the row, or ContextMenu /
  // Shift+F10 on it. Its Esc stays in the menu (the popover closes on any Esc that reaches the window).
  let menu = $state<{ userId: string; serverId: string | null; x: number; y: number; opener: HTMLElement } | null>(null)
  function openMenu(item: InboxItem, e: MouseEvent | KeyboardEvent, opener: HTMLElement) {
    const id = item.message.author_id
    if (!id || (e instanceof KeyboardEvent && !isMenuKey(e))) return
    e.preventDefault()
    menu = { userId: id, serverId: app.state.channels[item.message.channel_id]?.server_id ?? null, ...menuPoint(e, opener), opener }
  }

  /** Near the end of the list: fetch the next page. */
  function scrolled(e: Event) {
    const el = e.currentTarget as HTMLElement
    if (el.scrollHeight - el.scrollTop - el.clientHeight < 120) void loadMore(inbox.tab)
  }
</script>

<div class="inbox-pop menu-surface pop-in" role="dialog" aria-label="Inbox" use:clickOutside={onClose}>
  <div class="head">
    <h2>Inbox{#if inbox.unread > 0}<span class="count" aria-label="{inbox.unread} unread">{badgeText(inbox.unread)}</span>{/if}</h2>
    <button type="button" class="btn btn-quiet sm" disabled={inbox.unread === 0} onclick={() => void markAllRead()}>Mark all read</button>
  </div>
  <div class="tabs"><Segmented label="Show" options={TABS} value={inbox.tab} onchange={setTab} /></div>
  <div class="list" onscroll={scrolled}>
    {#if list?.items.length}
      {#each list.items as item (item.message.id)}
        {@const m = item.message}
        {@const reply = snippet(item)}
        <button type="button" class="item" class:unread={!item.read} onclick={() => openItem(item)}
          oncontextmenu={(e) => openMenu(item, e, e.currentTarget)} onkeydown={(e) => openMenu(item, e, e.currentTarget)}
          aria-label="{item.read ? '' : 'Unread: '}{item.kind === 'reply' ? 'reply' : 'mention'} from {nameIn(item, m.author_id)} in {whereLabel(app.state, item)}">
          <span class="av" aria-hidden="true"><Avatar id={m.author_id ?? ''} name={nameIn(item, m.author_id)} size={32} /></span>
          <span class="text" aria-hidden="true">
            <span class="top"><span class="author">{nameIn(item, m.author_id)}</span><span class="where">{whereLabel(app.state, item)}</span><time datetime={m.created_at} data-tip={new Date(m.created_at).toLocaleString()}>{relativeTime(m.created_at, nowMs)}</time></span>
            {#if reply}<span class="snippet" class:gone={reply.deleted}>Replying to {reply.who}: {reply.text}</span>{/if}
            <span class="said">{#each parts(item) as p}{#if p.me}<mark class="me">{p.text}</mark>{:else}{p.text}{/if}{/each}</span>
          </span>
        </button>
      {/each}
    {:else if list && !list.loading && !list.error}
      <p class="none">{inbox.tab === 'mentions' ? 'No mentions in the last 30 days.' : 'Nothing for you in the last 30 days. Mentions and replies to you land here.'}</p>
    {/if}
    {#if list?.error}
      <p class="error" role="alert">{list.error} <button type="button" class="link" onclick={() => void loadMore(inbox.tab)}>Try again</button></p>
    {:else if !list || list.loading}
      <Loading />
    {/if}
  </div>
  {#if menu}
    <!-- inside the popover (a press on it isn't outside); its Esc goes no further than the menu -->
    <div class="person-menu" role="presentation" onkeydown={(e) => { if (e.key === 'Escape') e.stopPropagation() }}>
      {#key menu}<MemberMenu serverId={menu.serverId} userId={menu.userId} x={menu.x} y={menu.y} opener={menu.opener} onMessage={messagePerson} onClose={() => (menu = null)} />{/key}
    </div>
  {/if}
</div>

<style>
  /* the pins popover's surface, dropped from the top bar's button on the right */
  .inbox-pop { position: absolute; top: calc(100% + 6px); right: 0; z-index: 60; width: min(420px, calc(100vw - 24px)); max-height: min(70vh, 640px); padding: 14px 6px 6px; gap: 8px; color: var(--text); }
  .person-menu { display: contents; }
  .head { display: flex; align-items: center; gap: 8px; padding: 0 8px; }
  h2 { flex: 1; margin: 0; font-size: 15px; display: flex; align-items: center; gap: 6px; }
  .count { min-width: 16px; height: 16px; border-radius: var(--radius-sm); background: var(--danger); color: var(--on-danger); font-size: 10px; font-weight: 700; display: inline-block; box-sizing: border-box; text-align: center; line-height: normal; text-box: trim-both cap alphabetic; padding: calc((16px - 1cap) / 2) 4px; }
  .tabs { padding: 0 8px; }
  .list { min-height: 0; overflow-y: auto; display: flex; flex-direction: column; gap: 2px; }
  .item { display: flex; align-items: flex-start; gap: 10px; width: 100%; padding: 8px; border: 0; border-radius: var(--radius-sm); background: transparent; color: inherit; font: inherit; text-align: left; }
  .item:hover, .item:focus-visible { background: var(--bg-4); }
  /* unread: as a message that mentions you is marked in chat */
  .item.unread { background: var(--accent-faint); box-shadow: inset 2px 0 0 var(--focus-ring); }
  .item.unread:hover, .item.unread:focus-visible { background: var(--bg-4); }
  .av { flex-shrink: 0; }
  .text { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px; font-size: 13px; }
  .top { display: flex; align-items: baseline; gap: 6px; min-width: 0; }
  .author { font-weight: 600; color: var(--text); white-space: nowrap; }
  .where { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--text-3); font-size: 12px; }
  time { flex-shrink: 0; color: var(--text-3); font-size: 12px; }
  .unread .author::after { content: ''; display: inline-block; width: 6px; height: 6px; margin-left: 6px; border-radius: 50%; background: var(--accent); vertical-align: middle; }
  .snippet { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--text-3); font-size: 12px; }
  .snippet.gone { font-style: italic; }
  .said { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--text-2); }
  mark.me { color: var(--accent-text); background: var(--accent-soft); border-radius: 4px; padding: 0 3px; font-weight: 600; }
  .sm { height: 26px; padding: 0 8px; font-size: 12px; }
  .none { margin: 6px 8px; color: var(--text-3); font-size: 13px; }
  .error { margin: 6px 8px; color: var(--danger-text); font-size: 13px; }
  .link { border: 0; padding: 0; background: none; color: var(--accent-text); font: inherit; text-decoration: underline; cursor: pointer; }
</style>
