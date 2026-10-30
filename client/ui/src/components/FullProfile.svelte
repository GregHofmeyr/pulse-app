<script lang="ts">
  // The full profile (spec §3.2, plan reconciliation 15): a centred modal with the card on the left and
  // Board panels on the right. The profile comes from the cache; a ProfileUpdated restyles it live.
  import { untrack } from 'svelte'
  import ProfileCard from './ProfileCard.svelte'
  import TrophyCase from './TrophyCase.svelte'
  import Name from './Name.svelte'
  import Icon from './Icon.svelte'
  import { modal } from '../lib/actions'
  import { app, loadProfile, revalidateProfile } from '../lib/store.svelte'
  import { userOf } from '../lib/selectors'
  import { DEFAULT_THEME, cardVars } from '../lib/cardTheme'
  import { twemojiUrl } from '../lib/emoji'
  import { animatedEmoji, stillSrc } from '../lib/animate.svelte'
  import { errorText } from '../lib/tauri'
  import { formatMemberSince, formatVoice } from '../lib/profileFormat'
  import type { CardProfile } from '../lib/protocol/CardProfile'

  let { userId, serverId, onClose }: { userId: string; serverId: string | null; onClose: () => void } = $props()

  const PLACEHOLDER: CardProfile = { banner_hash: null, banner_still: null, theme_top: DEFAULT_THEME.top, theme_bottom: DEFAULT_THEME.bottom, pronouns: null, bio: null }
  const cached = $derived(app.state.profiles[userId])
  const user = $derived(userOf(app.state, userId) ?? cached?.user ?? null)
  const card = $derived(cached?.card ?? PLACEHOLDER)
  const stats = $derived(cached?.stats ?? null)
  const vars = $derived(Object.entries(cardVars(card.theme_top, card.theme_bottom)).map(([k, v]) => `${k}: ${v}`).join('; '))
  let failed = $state<string | null>(null)

  function load(id: string) {
    failed = null
    loadProfile(id).catch((e) => {
      if (userId === id) failed = errorText(e)
    })
  }
  const has = $derived(!!cached)
  // Fetch when nothing is cached (shared with any load already in flight); a cached profile shows at
  // once and is refreshed in the background if it is over 5 minutes old.
  $effect(() => {
    const id = userId
    if (has) return untrack(() => revalidateProfile(id))
    untrack(() => load(id))
  })

  // Newest first, whatever order the server sent.
  const archive = $derived([...(cached?.name_archive ?? [])].sort((a, b) => b.changed_at.localeCompare(a.changed_at)))
  const serverName = (id: string) => app.state.servers.find((s) => s.id === id)?.name ?? 'a server'
  const customOf = (key: string) => app.state.customEmoji[key.slice(7)]
</script>

<div class="overlay">
  <div class="backdrop" role="presentation" data-backdrop onclick={onClose}></div>
  <div class="modal" role="dialog" aria-modal="true" aria-label="Profile of {user?.username ?? 'user'}" tabindex="-1" use:modal={onClose} style={vars}>
    <button class="close" aria-label="Close profile" onclick={onClose}><Icon name="x" size={16} /></button>
    <div class="left">
      {#if user}
        <ProfileCard {user} {card} badges={cached?.badges ?? []} {stats} {serverId} size="full" loading={!cached && !failed} {onClose} />
      {/if}
      {#if failed && !cached}<p class="err" role="alert">{failed} <button type="button" class="retry" onclick={() => load(userId)}>Retry</button></p>{/if}
    </div>
    <div class="right">
      {#if stats}
        <section class="board">
          <h2>Stats</h2>
          <dl>
            <div><dt>Voice</dt><dd>{formatVoice(stats.voice_seconds)}</dd></div>
            <div><dt>Member since</dt><dd>{formatMemberSince(stats.member_since)}</dd></div>
            {#if stats.top_reaction}
              {@const key = stats.top_reaction}
              {@const c = key.startsWith('custom:') ? customOf(key) : undefined}
              <div>
                <dt>Most-used reaction</dt>
                <dd class="react">
                  {#if !key.startsWith('custom:')}
                    <img src={twemojiUrl(key)} alt={key} draggable="false" />
                  {:else if c?.hash}
                    <img src={stillSrc(c)} alt={`:${c.name}:`} draggable="false" {@attach animatedEmoji(c)} />
                  {:else}
                    <span>:{c?.name ?? key}:</span>
                  {/if}
                </dd>
              </div>
            {/if}
            {#if stats.times_renamed > 0}<div><dt>Times renamed</dt><dd>{stats.times_renamed}</dd></div>{/if}
          </dl>
        </section>
      {/if}
      {#if archive.length > 0}
        <section class="board">
          <h2>Name Archive</h2>
          <ul class="archive">
            {#each archive as e, i (`${i}\t${e.changed_at}\t${e.name}\t${e.server_id}\t${e.changed_by}`)}
              <li>
                <strong>{e.name}</strong>
                <span>{serverName(e.server_id)}</span>
                <span>by <Name userId={e.changed_by} serverId={e.server_id} interactive={false} /></span>
                <time datetime={e.changed_at}>{formatMemberSince(e.changed_at)}</time>
              </li>
            {/each}
          </ul>
        </section>
      {/if}
      <section class="board">
        <h2>Trophy case</h2>
        <TrophyCase badges={cached?.badges ?? []} />
      </section>
    </div>
  </div>
</div>

<style>
  /* flex-centred (not transform): WebKitGTK blurs text on sub-pixel transforms. Above every other
     modal (it opens from names inside them, e.g. QOTY voting); the quick card sits above it
     (stacking.test.ts). */
  .overlay { position: fixed; inset: 0; z-index: 50; display: grid; place-items: center; padding: 32px 16px; }
  .backdrop { position: absolute; inset: 0; background: var(--scrim); }
  /* the Board background: the theme's tinted surface (spec §3.2) */
  .modal { position: relative; width: min(880px, calc(100vw - 32px)); height: min(600px, calc(100vh - 64px)); display: flex; gap: 16px; padding: 16px; background: linear-gradient(var(--card-top), var(--card-bottom)); border-radius: 16px; border: 1px solid var(--bg-3); overflow: hidden; outline: none; }
  /* padding: room for the card's glow (a scrolling box clips shadows at its edge), given back by the margin */
  .left { flex: 0 0 calc(340px + 52px); overflow-y: auto; padding: 26px; margin: -26px; }
  .right { flex: 1; min-width: 0; overflow-y: auto; display: flex; flex-direction: column; gap: 12px; padding-right: 4px; }
  .close { position: absolute; top: 12px; right: 12px; width: 32px; height: 32px; border: 1px solid var(--bg-4); border-radius: 50%; background: var(--bg-1); color: var(--text-2); display: grid; place-items: center; z-index: 1; }
  .close:hover { background: var(--bg-3); color: var(--text); }
  .board { border-radius: 12px; padding: 14px 16px; background: linear-gradient(var(--panel-top), var(--panel-bottom)); color: var(--card-text); }
  h2 { margin: 0 0 10px; font-size: 12px; font-weight: 700; letter-spacing: .06em; text-transform: uppercase; color: var(--card-text-2); }
  dl { margin: 0; display: grid; grid-template-columns: repeat(auto-fill, minmax(150px, 1fr)); gap: 10px; }
  dt { font-size: 12px; color: var(--card-text-2); }
  dd { margin: 2px 0 0; font-size: 16px; font-weight: 600; }
  .react img { width: 22px; height: 22px; vertical-align: middle; }
  .archive { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 8px; font-size: 13px; }
  .archive li { display: flex; flex-wrap: wrap; gap: 4px 12px; align-items: baseline; }
  .archive span, .archive time { color: var(--card-text-2); }
  .err { margin: 6px 0 0; padding: 6px 10px; border-radius: 8px; background: var(--bg-3); color: var(--text-2); font-size: 12px; }
  .retry { margin-left: 6px; padding: 2px 10px; border-radius: 6px; border: 1px solid var(--bg-4); background: var(--bg-2); color: var(--text); font: inherit; font-weight: 600; cursor: pointer; }
  @media (max-width: 760px) {
    .modal { flex-direction: column; overflow-y: auto; }
    .left { flex: none; }
    .right { overflow: visible; }
  }
</style>
