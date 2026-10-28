<script lang="ts">
  // A person's card (spec §3.1). Pure: everything comes from props, except names and avatars (which
  // read the store, so they follow a ProfileUpdated) and where they are in voice. Used by the quick
  // card popover, the full profile and the editor's preview.
  import type { Attachment } from 'svelte/attachments'
  import Avatar from './Avatar.svelte'
  import Name from './Name.svelte'
  import Icon from './Icon.svelte'
  import { app, openDm, ui } from '../lib/store.svelte'
  import { addPending } from '../lib/state'
  import { api, errorText } from '../lib/tauri'
  import { cardVars } from '../lib/cardTheme'
  import { mediaUrl } from '../lib/emoji'
  import { renderMarkdown } from '../lib/markdown'
  import { animateEmojiIn, shouldAnimateProfile, wireAnimated } from '../lib/animate.svelte'
  import { formatMemberSince, formatVoice } from '../lib/profileFormat'
  import { badgeInfo } from '../lib/badges'
  import { openFull } from '../lib/profileCard.svelte'
  import type { User } from '../lib/protocol/User'
  import type { CardProfile } from '../lib/protocol/CardProfile'
  import type { Badge } from '../lib/protocol/Badge'
  import type { ProfileStats } from '../lib/protocol/ProfileStats'

  let { user, card, badges, stats, serverId, size = 'quick', preview = false, loading = false, onViewFull, onClose }: {
    user: User
    card: CardProfile
    badges: Badge[]
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

  // The glow is each raw theme colour at 45% alpha (plan reconciliation 8).
  const GLOW_ALPHA = '73'
  const style = $derived(
    Object.entries(cardVars(card.theme_top, card.theme_bottom)).map(([k, v]) => `${k}: ${v}`).join('; ')
      + `; --glow-top-a: ${card.theme_top}${GLOW_ALPHA}; --glow-bottom-a: ${card.theme_bottom}${GLOW_ALPHA}`,
  )
  const mine = $derived(user.id === app.state.me?.id)
  // Their nickname in this server, else their username (never "unknown": the user is in hand).
  const name = $derived((serverId && app.state.members[serverId]?.find((m) => m.user.id === user.id)?.nickname) || user.username)
  const online = $derived(app.state.people[user.id]?.online ?? null)
  const bioHtml = $derived(card.bio ? renderMarkdown(card.bio, [], app.state.customEmoji) : '')
  // The voice room they're in, only if it is one of your channels (one you can see).
  const voiceRoom = $derived.by(() => {
    for (const [id, list] of Object.entries(app.state.voice)) {
      const ch = app.state.channels[id]
      if (ch && list.some((m) => m.user_id === user.id)) return ch.name
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
      const nonce = crypto.randomUUID()
      app.state = addPending(app.state, channelId, { nonce, content: text, reply_to_id: null, thread_root_id: null, status: 'pending', created_at: new Date().toISOString() })
      void api.sendMessage(channelId, text, null, nonce)
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
    <span class="ring"><Avatar id={user.id} name={user.username} size={80} place="card" {online} /></span>
    <div class="who">
      <Name userId={user.id} {serverId} text={name} interactive={false} size="card" />
      <div class="handle">{user.username}{card.pronouns ? ` · ${card.pronouns}` : ''}</div>
    </div>
    {#if badges.length}
      <div class="badges">
        {#each badges as b (b.key)}
          {@const info = badgeInfo(b.key)}
          <span class="badge" role="img" aria-label={info.name} title={info.name}><Icon name={info.icon} size={16} /></span>
        {/each}
      </div>
    {/if}
    {#if loading}
      <div class="panel skeleton" aria-hidden="true"><span></span><span></span></div>
    {:else if bioHtml}
      <!-- renderMarkdown output is DOMPurify-sanitised (the message renderer's allow-list) -->
      <div class="panel bio" {@attach animateEmojiIn(bioHtml)}>{@html bioHtml}</div>
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
    {#if !preview && size === 'quick'}
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
  .ring { align-self: flex-start; margin-top: -46px; position: relative; border-radius: 50%; padding: 6px; background: var(--card-top); }
  .ring :global(.pres) { border-color: var(--card-top); border-width: 3px; }
  .who { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  /* Name's card size is 1.15em: this makes it 20 px, bold */
  .who :global(.name) { font-weight: 700; }
  .who { font-size: calc(20px / 1.15); }
  .handle { font-size: 13px; color: var(--card-text-2); }
  .badges { display: flex; flex-wrap: wrap; gap: 6px; }
  .badge { display: grid; place-items: center; width: 26px; height: 26px; border-radius: 8px; background: var(--panel-top); color: var(--card-text); }
  .panel { border-radius: 10px; padding: 10px 12px; background: linear-gradient(var(--panel-top), var(--panel-bottom)); font-size: 13px; }
  .bio { line-height: 1.4; overflow-wrap: anywhere; }
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
  .full-link { align-self: flex-start; background: none; border: 0; padding: 0; color: var(--card-text); font: inherit; font-size: 13px; font-weight: 600; cursor: pointer; }
  .full-link:hover { text-decoration: underline; }
  .msg { height: 36px; border-radius: 10px; border: 1px solid rgb(242 243 246 / .14); background: var(--panel-bottom); color: var(--card-text); padding: 0 12px; font: inherit; font-size: 13px; }
  .msg::placeholder { color: var(--card-text-2); opacity: .75; }
  .err { margin: 0; font-size: 12px; color: #ff9d96; }
</style>
