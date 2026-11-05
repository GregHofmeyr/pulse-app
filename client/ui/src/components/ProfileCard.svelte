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
  import { app, clock, openDm, queueMessage, ui } from '../lib/store.svelte'
  import { presenceOf } from '../lib/presence'
  import { errorText } from '../lib/tauri'
  import { baseName, displayName } from '../lib/selectors'
  import { cardVars, nameStop } from '../lib/cardTheme'
  import { cardLook } from '../lib/theme.svelte'
  import { mediaUrl } from '../lib/emoji'
  import { renderMarkdown } from '../lib/markdown'
  import { animateEmojiIn, shouldAnimateProfile, wireAnimated } from '../lib/animate.svelte'
  import { formatMemberSince, formatVoice } from '../lib/profileFormat'
  import BadgeIcon from './BadgeIcon.svelte'
  import { badgeStore, fallbackName } from '../lib/badges.svelte'
  import { editProfile, openCardSettings, openFull } from '../lib/profileCard.svelte'
  import { menuPoint } from '../lib/participantMenu'
  import { nickItems } from '../lib/nicknameMenu'
  import MemberMenu from './MemberMenu.svelte'
  import StatusPicker from './StatusPicker.svelte'
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

  // The card's own two colours, drawn light or dark as the app theme is (#139, lib/cardTheme.ts).
  const vars = $derived(cardVars(card.theme_top, card.theme_bottom, cardLook()))
  const style = $derived(Object.entries(vars).map(([k, v]) => `${k}: ${v}`).join('; '))
  const nameBg = $derived(nameStop(vars['--card-top'], vars['--card-bottom'], cardLook().light))
  const mine = $derived(user.id === app.state.me?.id)
  // The name that applies here (#47): their nickname in this server, else your private nickname for
  // them, else their display name; the preview draws the display name being edited. The username
  // shows only as the @handle below it.
  const name = $derived(preview ? baseName(user) : displayName(app.state, serverId, user.id))
  // no dot for someone we know nothing of; yours shows your own status (Invisible: the grey offline dot, #171)
  const presence = $derived(mine || app.state.people[user.id] ? presenceOf(app.state, user.id, clock.nowIso) : null)
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

  // Your own card's status row (#45) opens your status menu, above it.
  let statusMenu = $state<HTMLElement | null>(null)

  // Every action between people lives in the right-click menu, never a button on the card (owner, #137): right-click
  // the card (or ContextMenu / Shift+F10 in it) for the person menu, without View profile since you're on it.
  let menu = $state<{ x: number; y: number; opener: HTMLElement } | null>(null)
  const hasMenu = $derived(!preview && nickItems(app.state, serverId, user.id).length > 0)
  function openMenu(e: MouseEvent | KeyboardEvent) {
    if (!hasMenu) return
    e.preventDefault()
    const card = e.currentTarget as HTMLElement
    const a = document.activeElement
    const opener = e instanceof KeyboardEvent && a instanceof HTMLElement && card.contains(a) ? a : card
    menu = { ...menuPoint(e, opener), opener }
  }

  function viewFull() {
    if (onViewFull) onViewFull()
    else openFull(user.id, serverId)
  }
</script>

<!-- the context menu is a pointer shortcut; the keyboard route is ContextMenu / Shift+F10 inside the card -->
<div class="card" class:full={size === 'full'} {style} role="presentation" oncontextmenu={openMenu}
  onkeydown={(e) => { if (e.key === 'ContextMenu' || (e.key === 'F10' && e.shiftKey)) openMenu(e) }}>
  <!-- loading: the full banner height, so the card does not grow when the profile arrives (#173) -->
  <div class="banner" class:none={!banner && !loading}>
    {#if banner}<img src={mediaUrl(bannerStill ?? banner)} alt="" draggable="false" {@attach wireBanner} />{/if}
  </div>
  <div class="body">
    {#if !preview && size === 'quick'}
      <!-- the avatar is a way in to the full profile too -->
      <button type="button" class="ring avatar-link" aria-label="View full profile" onclick={viewFull}><Avatar id={user.id} {name} size={80} place="card" {presence} /></button>
    {:else}
      <span class="ring"><Avatar id={user.id} {name} size={80} place="card" {presence} user={preview ? user : undefined} /></span>
    {/if}
    <div class="who">
      <!-- No server for the style: a card shows the person's own colour, never a role colour (spec §2) -->
      <Name userId={user.id} serverId={null} text={name} interactive={false} size="card" bg={nameBg} user={preview ? user : undefined} />
      <div class="handle">@{user.username}{card.pronouns ? ` · ${card.pronouns}` : ''}</div>
    </div>
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
    {#if loading && !stats}
      <!-- where Member since and Voice will be -->
      <div class="panel skeleton stats-skeleton" aria-hidden="true"><span></span><span></span></div>
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
      <!-- your own card: one box of actions, like Discord's; the avatar and Badges open the full profile. Only on
           yours: actions between people stay in the right-click menu (owner) -->
      <div class="actions">
        <button type="button" onclick={editProfile}><Icon name="pencil" size={16} /><span>Edit profile</span></button>
        <!-- your status (#45): the row opens the status menu -->
        <button type="button" aria-haspopup="menu" aria-expanded={!!statusMenu} data-menu-toggle onclick={(e) => (statusMenu = statusMenu ? null : e.currentTarget)}>
          <span class="status {presence?.kind ?? 'online'}"></span><span>{presence?.label ?? 'Online'}</span><Icon name="chevronRight" size={14} /></button>
        {#if size === 'quick'}<button type="button" onclick={viewFull}><Icon name="trophy" size={16} /><span>Badges</span></button>{/if}
        <!-- Settings (#168): the me area opens only this card, so the card is the way there (the cog stays too) -->
        <button type="button" onclick={openCardSettings}><Icon name="gear" size={16} /><span>Settings</span></button>
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
{#if statusMenu}
  <StatusPicker opener={statusMenu} onClose={() => (statusMenu = null)} />
{/if}
{#if menu}
  {#key menu}<MemberMenu {serverId} userId={user.id} x={menu.x} y={menu.y} opener={menu.opener} fromCard onClose={() => (menu = null)} />{/key}
{/if}

<style>
  /* Surface: the theme mixed into near-black (white on a light app theme, #139); border: the edge colours; glow: each at
     45% alpha (reconciliation 8); on a light theme a soft lift too. */
  .card {
    width: 300px; border-radius: 16px; overflow: hidden; color: var(--card-text);
    border: 1.5px solid transparent;
    background: linear-gradient(var(--card-top), var(--card-bottom)) padding-box, linear-gradient(var(--glow-top), var(--glow-bottom)) border-box;
    box-shadow: 0 -4px 22px var(--glow-top-a), 0 4px 22px var(--glow-bottom-a), var(--card-lift);
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
  .skeleton span { height: 10px; border-radius: 5px; background: var(--card-skeleton); }
  .skeleton span:last-child { width: 60%; }
  /* the stats panel's two 13px rows */
  .stats-skeleton { gap: 10px; padding: 14px 12px; }
  .info { margin: 0; display: flex; flex-direction: column; gap: 6px; }
  .info div { display: flex; justify-content: space-between; gap: 8px; }
  dt { color: var(--card-text-2); }
  dd { margin: 0; }
  .info .room { justify-content: flex-start; align-items: center; gap: 6px; color: var(--card-ok); }
  .avatar-link { border: 0; cursor: pointer; }
  .actions { display: flex; flex-direction: column; padding: 4px; border-radius: 10px; background: linear-gradient(var(--panel-top), var(--panel-bottom)); }
  .actions button { display: flex; align-items: center; gap: 10px; height: 36px; padding: 0 10px; border: 0; border-radius: 8px; background: none; color: var(--card-text); font: inherit; font-size: 13px; font-weight: 600; text-align: left; cursor: pointer; }
  /* between rows: a faint straight line, inset from both sides (the card's own line colour) */
  .actions button + button { position: relative; }
  .actions button + button::before { content: ''; position: absolute; top: 0; left: 10px; right: 10px; height: 1px; background: var(--card-line); pointer-events: none; }
  .actions button:hover { background: var(--card-hover); }
  .actions span:not(.status) { flex: 1; }
  /* the dot as Avatar draws it (Avatar.svelte .pres), on the panel */
  .status { position: relative; width: 10px; height: 10px; margin: 0 3px; border-radius: 50%; background: var(--ok); }
  .status.dnd { background: var(--danger); }
  .status.dnd::after { content: ''; position: absolute; left: 18%; right: 18%; top: 50%; height: 26%; transform: translateY(-50%); border-radius: 999px; background: var(--panel-top); }
  .status.invisible { background: var(--presence-off); }
  .full-link { align-self: flex-start; background: none; border: 0; padding: 0; color: var(--card-text); font: inherit; font-size: 13px; font-weight: 600; cursor: pointer; }
  .full-link:hover { text-decoration: underline; }
  .msg { height: 36px; border-radius: 10px; border: 1px solid var(--card-edge); background: var(--panel-bottom); color: var(--card-text); padding: 0 12px; font: inherit; font-size: 13px; }
  .msg::placeholder { color: var(--card-text-2); opacity: .75; }
  .err { margin: 0; font-size: 12px; color: var(--card-danger); }
</style>
