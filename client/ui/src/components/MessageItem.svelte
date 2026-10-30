<script lang="ts" module>
  export { INLINE_ERROR_MS } from '../lib/inlineError.svelte'
</script>

<script lang="ts">
  import { actionBarFocus } from '../lib/focus'
  import Icon from './Icon.svelte'
  import EmojiBox from './EmojiBox.svelte'
  import EmojiPopover from './EmojiPopover.svelte'
  import ReactionRow from './ReactionRow.svelte'
  import { reactionKey, type Std } from '../lib/emoji'
  import { pickAdds, react } from '../lib/reactions'
  import { untrack } from 'svelte'
  import { app } from '../lib/store.svelte'
  import { errorText } from '../lib/tauri'
  import { InlineError } from '../lib/inlineError.svelte'
  import type { CustomEmoji } from '../lib/protocol/CustomEmoji'
  import { renderMessage } from '../lib/markdown'
  import { animateEmojiIn } from '../lib/animate.svelte'
  import Avatar from './Avatar.svelte'
  import Name from './Name.svelte'
  import { mentionLookup, mentionName, styleMentions } from '../lib/mentionStyles'
  import { nameCss, resolveNameStyle } from '../lib/nameStyle'
  import { roleColorFor, userOf } from '../lib/selectors'
  import { appearance } from '../lib/appearance.svelte'
  import { openCard } from '../lib/profileCard.svelte'
  import type { Attachment } from 'svelte/attachments'
  import type { Message } from '../lib/protocol/Message'
  import type { Pending } from '../lib/state'

  let {
    message,
    serverId,
    name,
    nameOf,
    replyTo,
    grouped,
    mine,
    onReply,
    onEdit,
    onDelete,
    mentionNames = [],
    meId = '',
    pending,
    onRetry = () => {},
    onDiscard = () => {},
    onEditClosed = () => {},
  }: {
    message: Message
    /** The server the message is in (null in DMs and groups): names use its nicknames and role colours. */
    serverId: string | null
    name: string
    nameOf: (userId: string | null) => string
    replyTo: Message | null
    grouped: boolean
    mine: boolean
    onReply: () => void
    onEdit: (content: string) => Promise<void>
    onDelete: () => Promise<void>
    /** Usernames that may be mentioned here (rendered as pills). */
    mentionNames?: string[]
    meId?: string
    /** Set when `message` is an unsent one (`pendingMessage`): faded, no actions or reactions, status line under it. */
    pending?: Pending
    onRetry?: () => void
    onDiscard?: () => void
    /** The edit box closed (saved or cancelled): its focus has nowhere to go, so the caller gives it some. */
    onEditClosed?: () => void
  } = $props()
  const uid = $props.id()

  // --- names and avatars: the avatar animates while the pointer is over the row; pills take the person's style ---
  let rowHover = $state(false)
  // Pills restyle only when the HTML or a mentioned person's style changes, not on every state update:
  // the lookup is rebuilt only when people or members change, and the styles are compared as one string.
  const lookup = $derived(mentionLookup(app.state))
  const mentioned = $derived.by(() => {
    const c = message.content.toLowerCase()
    if (!c.includes('@')) return [] as string[]
    const ids = mentionNames.filter((n) => c.includes('@' + n.toLowerCase())).map(lookup)
    return [...new Set(ids.filter((id): id is string => !!id))]
  })
  const cssFor = (id: string) => nameCss(resolveNameStyle(userOf(app.state, id), { roleColor: roleColorFor(app.state, serverId, id), showStyles: appearance.showNameStyles }))
  const mentionCss = $derived(mentioned.map((id) => `${id}\t${cssFor(id)}`).join('\n'))
  // Only the emoji slice, so the {@html} below re-renders on an emoji change, not on every state update.
  const customEmoji = $derived(app.state.customEmoji)
  const mentionsIn = (_html: string, look: (name: string) => string | null, cssKey: string): Attachment<HTMLElement> => (node) => {
    if (!cssKey || !node.querySelector('.mention')) return
    const css = new Map(cssKey.split('\n').map((l) => l.split('\t') as [string, string]))
    styleMentions(node, (n) => { const id = look(n); return id && css.has(id) ? id : null }, (id) => css.get(id)!)
  }
  function openMention(e: MouseEvent | KeyboardEvent) {
    if (e instanceof KeyboardEvent && e.key !== 'Enter' && e.key !== ' ') return
    const pill = (e.target as Element).closest('.mention')
    const id = pill && lookup(mentionName(pill))
    if (!pill || !id) return
    if (e instanceof KeyboardEvent) e.preventDefault() // Space mustn't scroll
    openCard(id, serverId, pill)
  }

  let focusIn = $state(false)
  let picking = $state(false)
  let smileBtn: HTMLButtonElement | undefined = $state()
  let editing = $state(false)
  let draft = $state('')
  let editError = $state('')
  const inlineError = new InlineError()
  const showError = inlineError.show
  $effect(() => inlineError.stop)
  async function del() {
    try {
      await onDelete()
    } catch (e) {
      showError(errorText(e))
    }
  }
  const time = $derived(new Date(message.created_at).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }))

  let editBox: EmojiBox | undefined = $state()
  function startEdit() {
    draft = message.content
    editError = ''
    editing = true
  }
  // the box opens focused, the caret after the text
  $effect(() => {
    if (editBox) untrack(() => editBox!.focus(draft.length))
  })
  // a second Enter while the first save is in flight must not send the edit twice
  let saving = false
  async function save() {
    if (saving) return
    const d = draft.trim()
    editError = ''
    if (d && d !== message.content) {
      saving = true
      try {
        await onEdit(d)
      } catch (e) {
        editError = errorText(e) // the box stays open with the text intact
        return
      } finally {
        saving = false
      }
    }
    closeEdit()
  }
  function closeEdit() {
    // only focus the box had (or nobody has): a save landing after the person moved on leaves
    // their focus where it is
    const handBack = !!editBox?.hasFocus() || document.activeElement === document.body || !document.activeElement
    editing = false
    editError = ''
    if (handBack) onEditClosed()
  }
  function pick(e: Std | CustomEmoji, { keep }: { keep: boolean }) {
    if (!keep) picking = false
    const key = reactionKey(e)
    void react(() => app.state, (s) => (app.state = s), message.channel_id, message.id, key, pickAdds(message.reactions, key, keep), meId, showError)
  }
  // after the edit box's own keys (IME, undo/redo); Shift+Enter, not taken here, adds a line
  function key(e: KeyboardEvent) {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault()
      void save()
    } else if (e.key === 'Escape') {
      e.preventDefault()
      // a save in flight lands anyway: the box stays until it has (or shows why not)
      if (!saving) closeEdit()
    }
  }
</script>

{#snippet item()}
{#if message.kind === 'system'}
  <div class="system" data-message-id={message.id}>{message.content} <span class="time">{time}</span></div>
{:else}
  <!-- focus anywhere inside shows the action bar (hover alone left keyboard users without it) -->
  <div class="msg" data-message-id={message.id} {...actionBarFocus((v) => (focusIn = v))} class:grouped class:mentioned={!!meId && message.mentions.includes(meId)}
    onpointerenter={() => (rowHover = true)} onpointerleave={() => (rowHover = false)}>
    {#if replyTo}
      <div class="reply"><span class="hook"></span><strong>{#if replyTo.author_id}<Name userId={replyTo.author_id} {serverId} text={nameOf(replyTo.author_id)} />{:else}{nameOf(null)}{/if}</strong>
        <span class="snippet">{replyTo.deleted ? 'original message deleted' : replyTo.content.slice(0, 120)}</span></div>
    {:else if message.reply_to_id}
      <div class="reply"><span class="hook"></span><span class="snippet">original message not loaded</span></div>
    {/if}
    <div class="row">
      {#if grouped && !replyTo}
        <span class="gutter"></span>
      {:else}
        <Avatar id={message.author_id ?? ''} {name} size={40} interactive={!!message.author_id} {serverId} hovered={() => rowHover} />
      {/if}
      <div class="body">
        {#if !grouped || replyTo}<div class="meta"><strong>{#if message.author_id}<Name userId={message.author_id} {serverId} text={name} />{:else}{name}{/if}</strong><span class="time">{time}</span></div>{/if}
        {#if message.deleted}
          <p class="deleted">message deleted</p>
        {:else if editing}
          <EmojiBox bind:this={editBox} bind:text={draft} class="edit" label="Edit message" describedby="{uid}-hint" onkeydown={key} />
          <small class="hint" id="{uid}-hint">Enter to save · Esc to cancel · Shift+Enter for a new line</small>
          {#if editError}<p class="err" role="alert">{editError}</p>{/if}
        {:else}
          <!-- renderMessage output is DOMPurify-sanitised (allow-list, cached per message); the only {@html} in the app. -->
          {@const html = renderMessage(message, mentionNames, customEmoji)}
          <!-- a mention pill opens its person's card, by click or (focused: styleMentions makes known pills buttons) Enter or Space -->
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div class="content" {@attach animateEmojiIn(html)} {@attach mentionsIn(html, lookup, mentionCss)} onclick={openMention} onkeydown={openMention}>{@html html}</div>
          {#if message.edited_at}<span class="edited">(edited)</span>{/if}
        {/if}
        {#if !pending}<ReactionRow {message} {meId} {nameOf} onError={showError} />{/if}
        {#if inlineError.text}<p class="err" role="alert">{inlineError.text}</p>{/if}
      </div>
      {#if !pending && !message.deleted && !editing}
        <div class="actions" class:open={picking || focusIn}>
          <button bind:this={smileBtn} aria-label="Add reaction" data-menu-toggle onclick={() => (picking = !picking)}><Icon name="smile" size={15} /></button>
          <button aria-label="Reply" onclick={onReply}><Icon name="reply" size={15} /></button>
          {#if mine}
            <button aria-label="Edit" onclick={startEdit}><Icon name="pencil" size={15} /></button>
            <button aria-label="Delete" onclick={() => void del()}><Icon name="trash" size={15} /></button>
          {/if}
        </div>
        {#if picking && smileBtn}
          <EmojiPopover anchor={smileBtn} onPick={pick} onClose={() => (picking = false)} />
        {/if}
      {/if}
    </div>
  </div>
{/if}
{/snippet}

{#if pending}
  <div class="pend">
    <div class="unsent" style:opacity={0.6}>{@render item()}</div>
    <div class="status" class:failed={pending.status === 'failed'}>
      {#if pending.status === 'failed'}
        <span class="fail">Failed to send</span>
        <button onclick={onRetry}>Retry</button>
        <button onclick={onDiscard}>Discard</button>
      {:else}
        <span>Sending…</span>
      {/if}
    </div>
  </div>
{:else}
  {@render item()}
{/if}

<style>
  .status { display: flex; gap: 10px; align-items: baseline; padding: 0 18px 4px 72px; font-size: 12px; color: var(--text-3); }
  .status .fail { color: var(--danger); }
  .status button { border: 0; background: none; padding: 0; font-size: 12px; color: var(--text-2); text-decoration: underline; }
  .msg { padding: 8px 18px 2px; position: relative; }
  .msg.grouped { padding-top: 1px; }
  .msg:hover { background: rgba(255, 255, 255, .02); }
  .row { display: flex; gap: 14px; }
  .gutter { width: 40px; flex-shrink: 0; }
  .body { flex: 1; min-width: 0; }
  .meta { display: flex; align-items: baseline; gap: 8px; margin-bottom: 2px; }
  .meta strong { font-size: 14.5px; }
  .time { font-size: 11px; color: var(--text-3); }
  .content { color: var(--text-body); line-height: 1.5; overflow-wrap: anywhere; }
  .content :global(p) { margin: 0; }
  .content :global(code) { font-family: 'JetBrains Mono', ui-monospace, monospace; font-size: 12.5px; background: var(--bg-inset); border: 1px solid var(--line); border-radius: 6px; padding: 1px 6px; }
  .content :global(pre) { background: var(--bg-inset); border-radius: 8px; padding: 10px; overflow-x: auto; }
  .content :global(pre code) { border: 0; padding: 0; }
  .content :global(a) { color: var(--accent); }
  .content :global(blockquote) { margin: 4px 0; padding-left: 10px; border-left: 3px solid var(--bg-4); color: var(--text-2); }
  .edited { font-size: 11px; color: var(--text-3); }
  .deleted { margin: 0; font-style: italic; color: var(--text-3); }
  .reply { display: flex; align-items: center; gap: 6px; padding-left: 20px; margin-bottom: 4px; font-size: 13px; color: var(--text-3); min-width: 0; }
  .reply strong { color: var(--text-2); }
  .hook { width: 28px; height: 10px; margin-top: 10px; border-left: 2px solid var(--bg-4); border-top: 2px solid var(--bg-4); border-top-left-radius: 6px; flex-shrink: 0; }
  .snippet { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .actions { position: absolute; top: -10px; right: 18px; display: flex; opacity: 0; pointer-events: none; background: var(--bg-3); border: 1px solid var(--bg-4); border-radius: 8px; }
  .msg:hover .actions, .actions.open { opacity: 1; pointer-events: auto; }
  .actions button { width: 30px; height: 28px; border: 0; background: transparent; color: var(--text-2); display: grid; place-items: center; }
  .actions button:hover { color: var(--text); }
  /* about two lines to start, grows with what's in it (images included) up to 200px, then scrolls */
  .body :global(.edit) { min-height: 58px; max-height: 200px; border-radius: 8px; border: 1px solid var(--bg-4); background: var(--bg-1); padding: 8px; }
  .body :global(.edit:focus-visible) { border-color: var(--accent); }
  .err { margin: 2px 0 0; font-size: 12px; color: var(--danger); }
  .hint { font-size: 11px; color: var(--text-3); }
  .system { padding: 6px 18px 6px 30px; font-size: 13px; color: var(--text-3); }
  .msg.mentioned { background: rgba(139, 156, 255, .08); box-shadow: inset 2px 0 0 var(--accent); }
  .content :global(img.emoji) { width: 1.375em; height: 1.375em; vertical-align: -0.3em; object-fit: contain; }
  .content :global(img.emoji.big) { width: 48px; height: 48px; }
  .content :global(.emoji-gone) { color: var(--text-3); background: var(--bg-3); border-radius: 4px; padding: 0 4px; font-size: .9em; }
  .content :global(.emoji-gone.big) { font-size: 1.25em; }
  .content :global(.mention) { color: var(--accent); background: var(--accent-soft); border-radius: 4px; padding: 0 3px; font-weight: 600; font-size: calc(1em * var(--name-scale, 1)); line-height: calc(1.5em / var(--name-scale, 1)); vertical-align: baseline; cursor: pointer; }
  /* a gradient name clips its background to the text: the pill background is on this wrapper instead */
  .content :global(.mention-wrap) { background: var(--accent-soft); border-radius: 4px; padding: 0 3px; }
</style>
