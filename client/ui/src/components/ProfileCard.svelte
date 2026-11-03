<script lang="ts">
  // A person's card (spec §3.1). Pure: everything comes from props, except names and avatars (which
  // read the store, so they follow a ProfileUpdated, except in the preview, which draws `user`) and
  // where they are in voice. Used by the quick
  // card popover, the full profile and the editor's preview.
  import { untrack } from 'svelte'
  import type { Attachment } from 'svelte/attachments'
  import Avatar from './Avatar.svelte'
  import Name from './Name.svelte'
  import Icon from './Icon.svelte'
  import { app, openDm, queueMessage, ui } from '../lib/store.svelte'
  import { api, errorText } from '../lib/tauri'
  import { baseName, displayName, serverNick } from '../lib/selectors'
  import { canServerNick, nickLabel } from '../lib/nicknameMenu'
  import { cardVars } from '../lib/cardTheme'
  import { mediaUrl } from '../lib/emoji'
  import { renderMarkdown } from '../lib/markdown'
  import { animateEmojiIn, shouldAnimateProfile, wireAnimated } from '../lib/animate.svelte'
  import { formatMemberSince, formatVoice } from '../lib/profileFormat'
  import BadgeIcon from './BadgeIcon.svelte'
  import { badgeStore, fallbackName } from '../lib/badges.svelte'
  import { editProfile, openFull, openRename } from '../lib/profileCard.svelte'
  import type { User } from '../lib/protocol/User'
  import type { CardProfile } from '../lib/protocol/CardProfile'
  import type { Badge } from '../lib/protocol/Badge'
  import type { ProfileStats } from '../lib/protocol/ProfileStats'

  let { user, card, badges, showcase = [], stats, serverId, size = 'quick', preview = false, loading = false, onViewFull, onClose }: {
    user: User
    card: CardProfile
    badges: Badge[]
    /** The badges shown off on the card (#49), keys in `badges`, in order. */
    showcase?: string[]
    stats: ProfileStats | null
    serverId: string | null
    size?: 'quick' | 'full'
    /** The editor's preview: no actions. */
    preview?: boolean
    /** The profile is still being fetched: the bio shows a placeholder. */
    loading?: boolean
    onViewFull?: () => void
    onClose?: () => void
  } = $props()

  // The showcase, in its order; names and icons come from the catalogue (fetched once, on first need).
  const shown = $derived(showcase.map((k) => badges.find((b) => b.key === k)).filter((b): b is Badge => !!b))
  const wantsCatalogue = $derived(shown.length > 0)
  $effect(() => {
    if (wantsCatalogue) untrack(() => void badgeStore.load())
  })

  // The glow is each raw theme colour at 45% alpha (plan reconciliation 8).
  const GLOW_ALPHA = '73'
  const style = $derived(
    Object.entries(cardVars(card.theme_top, card.theme_bottom)).map(([k, v]) => `${k}: ${v}`).join('; ')
      + `; --glow-top-a: ${card.theme_top}${GLOW_ALPHA}; --glow-bottom-a: ${card.theme_bottom}${GLOW_ALPHA}`,
  )
  const mine = $derived(user.id === app.state.me?.id)
  // The name that applies here (#47): their nickname in this server, else your private nickname for
  // them, else their display name; the preview draws the display name being edited. The username
  // shows only as the @handle below it.
  const name = $derived(preview ? baseName(user) : displayName(app.state, serverId, user.id))
  // Renaming (#48): a server nickname needs you and them in this server; a private one is for others.
  // The same rules as a person's menu (#113, lib/nicknameMenu).
  const canNick = $derived(!preview && canServerNick(app.state, serverId, user.id))
  const nick = $derived(serverId ? serverNick(app.state, serverId, user.id) : null)
  const privateNick = $derived(app.state.privateNicks[user.id] ?? null)
  let nameError = $state<string | null>(null)
  async function clear(f: () => Promise<unknown>) {
    nameError = null
    try {
      await f()
    } catch (e) {
      nameError = errorText(e)
    }
  }
  const online = $derived(app.state.people[user.id]?.online ?? null)
  const bioHtml = $derived(card.bio ? renderMarkdown(card.bio, [], app.state.customEmoji) : '')
  // The server voice room they're in, only if it is one of your channels (one you can see); a
  // group call is never named.
  const voiceRoom = $derived.by(() => {
    for (const [id, list] of Object.entries(app.state.voice)) {
      const ch = app.state.channels[id]
      if (ch?.server_id && ch.kind === 'voice' && list.some((m) => m.user_id === user.id)) return ch.name
    }
    return null
  })

  const banner = $derived(card.banner_hash)
  const bannerStill = $derived(card.banner_still)
  const wireBanner = $derived<Attachment<HTMLImageElement>>((img) => {
    if (banner && bannerStill && bannerStill !== banner) return wireAnimated(img, mediaUrl(banner), mediaUrl(bannerStill), (s) => shouldAnimateProfile('card', s))
  })

  let draft = $state('')
  let sending = $state(false)
  let error = $state<string | null>(null)

  // Enter sends to your DM with them (created if needed), queued in the outbox; then the app shows that DM (reconciliation 19).
  async function send() {
    const text = draft.trim()
    if (!text || sending) return
    sending = true
    error = null
    try {
      const channelId = await openDm(user.id)
      queueMessage(channelId, text, null)
      draft = ''
      ui.goTo = channelId
      onClose?.()
    } catch (e) {
      error = errorText(e)
    } finally {
      sending = false
    }
  }

  function viewFull() {
    if (onViewFull) onViewFull()
    else openFull(user.id, serverId)
  }
</script>

<div class="card" class:full={size === 'full'} {style}>
  <div class="banner" class:none={!banner}>
    {#if banner}<img src={mediaUrl(bannerStill ?? banner)} alt="" draggable="false" {@attach wireBanner} />{/if}
  </div>
  <div class="body">
    {#if !preview && size === 'quick'}
      <!-- the avatar is a way in to the full profile too -->
      <button type="button" class="ring avatar-link" aria-label="View full profile" onclick={viewFull}><Avatar id={user.id} {name} size={80} place="card" {online} /></button>
    {:else}
      <span class="ring"><Avatar id={user.id} {name} size={80} place="card" {online} user={preview ? user : undefined} /></span>
    {/if}
    <div class="who">
      <!-- No server for the style: a card shows the person's own colour, never a role colour (spec §2) -->
      <Name userId={user.id} serverId={null} text={name} interactive={false} size="card" surface="island" user={preview ? user : undefined} />
      <div class="handle">@{user.username}{card.pronouns ? ` · ${card.pronouns}` : ''}</div>
    </div>
    {#if canNick || (!preview && !mine)}
      <div class="names">
        {#if canNick && serverId}
          <button type="button" onclick={() => openRename({ kind: 'server', userId: user.id, serverId })}>{nickLabel('nick', !!privateNick)}</button>
          {#if nick}<button type="button" onclick={() => clear(() => api.setNickname(serverId, user.id, null))}>{nickLabel('nick-reset', !!privateNick)}</button>{/if}
        {/if}
        {#if !preview && !mine}
          <button type="button" onclick={() => openRename({ kind: 'private', userId: user.id })}>{nickLabel('private-nick', !!privateNick)}</button>
          {#if privateNick}<button type="button" onclick={() => clear(() => api.setPrivateNickname(user.id, null))}>{nickLabel('private-nick-clear', !!privateNick)}</button>{/if}
        {/if}
      </div>
      {#if nameError}<p class="err" role="alert">{nameError}</p>{/if}
    {/if}
    {#if shown.length}
      <div class="badges">
        {#each shown as b (b.key)}
          {@const d = badgeStore.def(b.key)}
          <BadgeIcon icon={d?.icon ?? '🏆'} rarity={b.rarity} tier={b.tier} tiers={d?.tiers.length ?? 1} first={b.first_earner} hidden={b.hidden} size="show"
            label={d?.name ?? (b.hidden ? '???' : fallbackName(b.key))} />
        {/each}
      </div>
    {/if}
    {#if loading}
      <div class="panel skeleton" aria-hidden="true"><span></span><span></span></div>
    {:else if bioHtml}
      <!-- renderMarkdown output is DOMPurify-sanitised (the message renderer's allow-list) -->
      <div class="bio" {@attach animateEmojiIn(bioHtml)}>{@html bioHtml}</div>
    {/if}
    {#if stats || voiceRoom}
      <dl class="panel info">
        {#if stats}
          <div><dt>Member since</dt><dd>{formatMemberSince(stats.member_since)}</dd></div>
          <div><dt>Voice</dt><dd>{formatVoice(stats.voice_seconds)}</dd></div>
        {/if}
        {#if voiceRoom}<div class="room"><Icon name="speaker" size={14} /><span>In {voiceRoom}</span></div>{/if}
      </dl>
    {/if}
    {#if !preview && mine}
      <!-- your own card: one box of actions, like Discord's; the avatar and Badges open the full profile -->
      <div class="actions">
        <button type="button" onclick={editProfile}><Icon name="pencil" size={16} /><span>Edit profile</span></button>
        <!-- online / do not disturb / invisible (and invisible to chosen people) aren't built yet -->
        <button type="button" aria-disabled="true" data-tip="Status options are coming soon"><span class="status"></span><span>Online</span><Icon name="chevronRight" size={14} /></button>
        {#if size === 'quick'}<button type="button" onclick={viewFull}><Icon name="trophy" size={16} /><span>Badges</span></button>{/if}
      </div>
    {:else if !preview && size === 'quick'}
      <button type="button" class="full-link" onclick={viewFull}>View full profile</button>
    {/if}
    {#if !preview && !mine}
      <input class="msg" type="text" placeholder="Message @{name}" aria-label="Message @{name}" bind:value={draft} disabled={sending}
        onkeydown={(e) => { if (e.key === 'Enter' && !e.isComposing) { e.preventDefault(); void send() } }} />
      {#if error}<p class="err" role="alert">{error}</p>{/if}
    {/if}
  </div>
</div>

<style>
  /* Surface: the theme mixed into near-black; border: the raw colours; glow: each at 45% alpha (reconciliation 8). */
  .card {
    width: 300px; border-radius: 16px; overflow: hidden; color: var(--card-text);
    border: 1.5px solid transparent;
    background: linear-gradient(var(--card-top), var(--card-bottom)) padding-box, linear-gradient(var(--glow-top), var(--glow-bottom)) border-box;
    box-shadow: 0 -4px 22px var(--glow-top-a), 0 4px 22px var(--glow-bottom-a);
  }
  .card.full { width: 340px; }
  .banner { position: relative; height: 104px; }
  .card.full .banner { height: 118px; }
  .banner.none { height: 56px; }
  .banner img { width: 100%; height: 100%; object-fit: cover; display: block; }
  /* the banner fades into the card over its bottom 40% */
  .banner::after { content: ''; position: absolute; inset: 0; background: linear-gradient(to bottom, transparent 60%, var(--card-top)); pointer-events: none; }
  .body { padding: 0 16px 16px; display: flex; flex-direction: column; gap: 10px; }
  /* a thin cut-out round the avatar, and a status dot about a fifth of it (Discord's proportions) */
  .ring { align-self: flex-start; margin-top: -44px; position: relative; border-radius: 50%; padding: 4px; background: var(--card-top); }
  .ring :global(.pres) { width: 24%; height: 24%; right: 2px; bottom: 2px; border-color: var(--card-top); border-width: 3px; }
  .who { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  /* renaming: small text buttons under the name */
  .names { display: flex; flex-wrap: wrap; gap: 4px 12px; margin-top: -4px; }
  .names button { padding: 0; border: 0; background: none; color: var(--card-text-2); font: inherit; font-size: 12px; cursor: pointer; }
  .names button:hover, .names button:focus-visible { color: var(--card-text); text-decoration: underline; }
  /* Name's card size is 1.15em: this makes it 20 px, bold */
  .who :global(.name) { font-weight: 700; }
  .who { font-size: calc(20px / 1.15); }
  .handle { font-size: 13px; color: var(--card-text-2); }
  .badges { display: flex; flex-wrap: wrap; gap: 8px; padding: 2px 0; }
  /* a touch smaller than the shared showcase size; zoom keeps the ring, glyph, spark and tier in proportion */
  .badges :global(.badge-icon.show) { zoom: .84; }
  .panel { border-radius: 10px; padding: 10px 12px; background: linear-gradient(var(--panel-top), var(--panel-bottom)); font-size: 13px; }
  .bio { font-size: 13px; line-height: 1.4; overflow-wrap: anywhere; }
  .bio :global(p) { margin: 0; }
  .bio :global(p + p) { margin-top: 4px; }
  .skeleton { display: flex; flex-direction: column; gap: 8px; }
  .skeleton span { height: 10px; border-radius: 5px; background: rgb(242 243 246 / .12); }
  .skeleton span:last-child { width: 60%; }
  .info { margin: 0; display: flex; flex-direction: column; gap: 6px; }
  .info div { display: flex; justify-content: space-between; gap: 8px; }
  dt { color: var(--card-text-2); }
  dd { margin: 0; }
  .info .room { justify-content: flex-start; align-items: center; gap: 6px; color: var(--ok); }
  .avatar-link { border: 0; cursor: pointer; }
  .actions { display: flex; flex-direction: column; padding: 4px; border-radius: 10px; background: linear-gradient(var(--panel-top), var(--panel-bottom)); }
  .actions button { display: flex; align-items: center; gap: 10px; height: 36px; padding: 0 10px; border: 0; border-radius: 8px; background: none; color: var(--card-text); font: inherit; font-size: 13px; font-weight: 600; text-align: left; cursor: pointer; }
  /* between rows: a faint straight line, inset from both sides (the island's own grey, as in every theme) */
  .actions button + button { position: relative; }
  .actions button + button::before { content: ''; position: absolute; top: 0; left: 10px; right: 10px; height: 1px; background: rgb(242 243 246 / .08); pointer-events: none; }
  .actions button:hover:not([aria-disabled='true']) { background: rgb(242 243 246 / .08); }
  .actions button[aria-disabled='true'] { cursor: default; }
  .actions span:not(.status) { flex: 1; }
  .status { width: 10px; height: 10px; margin: 0 3px; border-radius: 50%; background: var(--ok); }
  .full-link { align-self: flex-start; background: none; border: 0; padding: 0; color: var(--card-text); font: inherit; font-size: 13px; font-weight: 600; cursor: pointer; }
  .full-link:hover { text-decoration: underline; }
  .msg { height: 36px; border-radius: 10px; border: 1px solid rgb(242 243 246 / .14); background: var(--panel-bottom); color: var(--card-text); padding: 0 12px; font: inherit; font-size: 13px; }
  .msg::placeholder { color: var(--card-text-2); opacity: .75; }
  .err { margin: 0; font-size: 12px; color: #ff9d96; }
</style>
