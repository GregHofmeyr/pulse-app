<script lang="ts">
  // The trophy case panel's body (#49, spec "Showing badges"): earned badges first (rarest first), then
  // the locked ones greyed with how to earn them; hidden ones you don't hold are "???" with their rarity, last,
  // hand-given ones show only when held. Progress is the viewer's own, so only your own case has bars.
  // Your own case also edits the showcase, and opening it marks your badges seen ([NEW] stays while open).
  import { untrack } from 'svelte'
  import BadgeIcon from './BadgeIcon.svelte'
  import { app, loadProfile } from '../lib/store.svelte'
  import { api, errorText } from '../lib/tauri'
  import { SHOWCASE_MAX, badgeStore, byRarity, defaultShowcase, fallbackName, progressText, rarityLabel, rarityVar, shareText } from '../lib/badges.svelte'
  import { formatMemberSince } from '../lib/profileFormat'
  import type { Badge } from '../lib/protocol/Badge'
  import type { BadgeDef } from '../lib/protocol/BadgeDef'

  // showcaseChosen: the owner saved a showcase of their own (false: it's the rarest-five default).
  let { badges, showcase = [], showcaseChosen = false, mine = false }: { badges: Badge[]; showcase?: string[]; showcaseChosen?: boolean; mine?: boolean } = $props()

  $effect(() => void untrack(() => badgeStore.load()))

  const cat = $derived(badgeStore.catalogue)
  const held = $derived(new Map(badges.map((b) => [b.key, b])))
  const earned = $derived([...badges].sort(byRarity))
  /** A hidden badge the viewer doesn't hold: the catalogue masks it as `hidden:<n>`, shown as "???". */
  const masked = (d: BadgeDef) => d.hidden && d.key.startsWith('hidden:')
  // the "???" ones go last (#131), each group in the catalogue's order
  const locked = $derived.by(() => {
    const all = (cat?.badges ?? []).filter((d) => !held.has(d.key) && !d.hand_given)
    return [...all.filter((d) => !masked(d)), ...all.filter(masked)]
  })

  const nameOf = (b: Badge, d: BadgeDef | undefined) => d?.name ?? (b.hidden ? '???' : fallbackName(b.key))
  const iconOf = (b: Badge, d: BadgeDef | undefined) => d?.icon ?? '🏆'
  const share = (d: BadgeDef | undefined, tier: number) => (cat && d?.tiers[tier - 1] ? shareText(d.tiers[tier - 1].holders, cat.people) : '')
  /** The bar toward the next tier: your own case, a counter badge, a tier still to reach. */
  const progressOf = (d: BadgeDef | undefined, tier: number) => {
    if (!mine || !d || d.progress === null || d.hand_given || tier >= d.tiers.length) return null
    const target = d.tiers[tier]?.threshold ?? 0
    return target > 0 ? { value: Math.min(d.progress, target), max: target, text: progressText(d, d.progress), label: `${d.name} progress` } : null
  }
  /** Combo (#191): each stat toward the next tier you haven't got, as a bar like the one above; none once all are. */
  const partsOf = (d: BadgeDef | undefined) =>
    mine && d ? (d.parts ?? []).map((p) => ({ value: Math.min(p.value, p.target), max: p.target, text: `${p.label} — ${p.value.toLocaleString('en-US')} / ${p.target.toLocaleString('en-US')}`, label: `${d.name} progress: ${p.label}` })) : []

  // [NEW]: what was unseen when it reached this case stays tagged while it's open.
  let fresh = $state<string[]>([])
  $effect(() => {
    if (!mine) return
    const unseen = badges.filter((b) => !b.seen)
    untrack(() => {
      if (unseen.length === 0 && badgeStore.bumped === 0) return
      fresh = [...new Set([...fresh, ...unseen.map((b) => b.key)])]
      badgeStore.markSeen(unseen)
    })
  })

  // The showcase editor: up to 5 held badges, in order. "Rarest five" goes back to the server's default,
  // and a default showcase stays the default (it follows later unlocks) until you change something.
  let editing = $state(false)
  let chosen = $state<string[]>([])
  let auto = $state(false)
  let saving = $state(false)
  let saveError = $state<string | null>(null)
  function edit() {
    chosen = showcase.filter((k) => held.has(k))
    auto = !showcaseChosen
    saveError = null
    editing = true
  }
  function toggle(key: string) {
    auto = false
    if (chosen.includes(key)) chosen = chosen.filter((k) => k !== key)
    else if (chosen.length < SHOWCASE_MAX) chosen = [...chosen, key]
  }
  function move(i: number, by: -1 | 1) {
    const j = i + by
    if (j < 0 || j >= chosen.length) return
    const next = [...chosen]
    ;[next[i], next[j]] = [next[j], next[i]]
    chosen = next
    auto = false
  }
  function rarest() {
    chosen = defaultShowcase(badges)
    auto = true
  }
  async function save() {
    saving = true
    saveError = null
    try {
      await api.setBadgeShowcase(auto ? [] : chosen)
      editing = false
      const me = app.state.me
      if (me) loadProfile(me.id).catch(() => {})
    } catch (e) {
      saveError = errorText(e)
    } finally {
      saving = false
    }
  }
</script>

{#if mine && earned.length > 0}
  <div class="showcase-edit">
    {#if editing}
      <p class="hint">Pick up to {SHOWCASE_MAX} for your card, in order.</p>
      <ol class="chosen">
        {#each chosen as key, i (key)}
          {@const b = held.get(key)!}
          {@const d = badgeStore.def(key)}
          <li data-key={key}>
            <BadgeIcon icon={iconOf(b, d)} rarity={b.rarity} tier={b.tier} tiers={d?.tiers.length ?? 1} first={b.first_earner} hidden={b.hidden} size="show" label={nameOf(b, d)} />
            <button type="button" class="mini" aria-label={`Move ${nameOf(b, d)} left`} disabled={i === 0} onclick={() => move(i, -1)}>‹</button>
            <button type="button" class="mini" aria-label={`Move ${nameOf(b, d)} right`} disabled={i === chosen.length - 1} onclick={() => move(i, 1)}>›</button>
          </li>
        {:else}
          <li class="none">Nothing chosen</li>
        {/each}
      </ol>
      <div class="actions">
        <button type="button" class="btn" onclick={rarest}>Rarest five</button>
        <button type="button" class="btn" onclick={() => (editing = false)}>Cancel</button>
        <button type="button" class="btn btn-primary" disabled={saving} onclick={save}>Save</button>
      </div>
      {#if saveError}<p class="err" role="alert">{saveError}</p>{/if}
    {:else}
      <button type="button" class="btn" onclick={edit}>Edit showcase</button>
    {/if}
  </div>
{/if}

{#if earned.length === 0}
  <p class="empty">No trophies yet</p>
{/if}
{#if earned.length > 0 || locked.length > 0}
  <ul class="grid">
    {#each earned as b (b.key)}
      {@const d = badgeStore.def(b.key)}
      {@const p = progressOf(d, b.tier)}
      {@const at = chosen.indexOf(b.key)}
      <li class="tile" style:--ring-color={rarityVar(b.rarity)}>
        <BadgeIcon icon={iconOf(b, d)} rarity={b.rarity} tier={b.tier} tiers={d?.tiers.length ?? 1} first={b.first_earner} hidden={b.hidden} isNew={fresh.includes(b.key)} label={nameOf(b, d)} />
        <span class="nm">{nameOf(b, d)}</span>
        <span class="rar">{rarityLabel(b.rarity)}{#if b.count > 1}<span class="stack"> ×{b.count}</span>{/if}</span>
        {#if b.first_earner}<span class="first">First!</span>{/if}
        {#if d?.description}<span class="desc">{d.description}</span>{/if}
        {#if p}<progress max={p.max} value={p.value} aria-label={p.label}></progress><span class="prog">{p.text}</span>{/if}
        {@render parts(d)}
        <!-- a masked badge on someone else's profile carries no date -->
        {#if b.earned_at}<span class="when">{formatMemberSince(b.earned_at)}</span>{/if}
        {#if share(d, b.tier)}<span class="share">{share(d, b.tier)}</span>{/if}
        {#if editing}
          <button type="button" class="pick" aria-pressed={at >= 0} disabled={at < 0 && chosen.length >= SHOWCASE_MAX} onclick={() => toggle(b.key)}>{at >= 0 ? `#${at + 1} · Remove` : 'Add to showcase'}</button>
        {/if}
      </li>
    {/each}
    {#each locked as d (d.key)}
      {@const p = progressOf(d, 0)}
      <li class="tile locked" style:--ring-color={rarityVar(d.tiers[0]?.rarity ?? 'common')}>
        <BadgeIcon icon={d.icon} rarity={d.tiers[0]?.rarity ?? 'common'} hidden={masked(d)} label={d.name} />
        <span class="nm">{d.name}</span>
        <span class="rar">{rarityLabel(d.tiers[0]?.rarity ?? 'common')}</span>
        <span class="desc">{d.description || 'Hidden until earned'}</span>
        {#if p}<progress max={p.max} value={p.value} aria-label={p.label}></progress><span class="prog">{p.text}</span>{/if}
        {@render parts(d)}
        {#if share(d, 1)}<span class="share">{share(d, 1)}</span>{/if}
      </li>
    {/each}
  </ul>
{/if}

{#snippet parts(d: BadgeDef | undefined)}
  {@const list = partsOf(d)}
  {#if list.length}
    <ul class="parts">
      {#each list as part (part.label)}<li class="part"><progress max={part.max} value={part.value} aria-label={part.label}></progress><span class="prog">{part.text}</span></li>{/each}
    </ul>
  {/if}
{/snippet}

<style>
  .empty { margin: 0 0 10px; color: var(--card-text-2); }
  .grid { list-style: none; margin: 0; padding: 0; display: grid; grid-template-columns: repeat(auto-fill, minmax(130px, 1fr)); gap: 10px; }
  .tile { display: flex; flex-direction: column; align-items: center; gap: 3px; text-align: center; padding: 12px 8px 10px; border-radius: var(--radius-md); background: var(--panel-bottom); }
  .tile :global(.badge-icon) { margin-bottom: 6px; }
  .locked { opacity: .55; filter: grayscale(.7); }
  .nm { font-weight: 600; color: var(--card-text); }
  .rar { font-size: 11px; font-weight: 700; letter-spacing: .04em; text-transform: uppercase; color: var(--ring-color); }
  .stack { color: var(--card-text); }
  .first { font-size: 11px; font-weight: 700; color: var(--first-gold); }
  .desc { font-size: 12px; color: var(--card-text-2); }
  .when, .share, .prog { font-size: 11px; color: var(--card-text-2); }
  /* Combo's stats (#191): one bar and line each, spaced as the tile's own lines */
  .parts { list-style: none; margin: 0; padding: 0; width: 100%; display: flex; flex-direction: column; gap: 3px; }
  .part { display: flex; flex-direction: column; align-items: center; gap: 3px; }
  progress { width: 100%; height: 5px; margin-top: 4px; appearance: none; border: 0; border-radius: 3px; overflow: hidden; background: var(--panel-top); }
  progress::-webkit-progress-bar { background: var(--panel-top); }
  progress::-webkit-progress-value { background: var(--ring-color); }
  progress::-moz-progress-bar { background: var(--ring-color); }
  .pick { margin-top: 6px; padding: 3px 8px; border-radius: var(--radius-xs); border: 1px solid var(--bg-4); background: var(--bg-2); color: var(--text); font: inherit; font-size: 11px; font-weight: 600; cursor: pointer; }
  .pick[aria-pressed='true'] { border-color: var(--focus-ring); color: var(--accent-text); }
  .showcase-edit { margin: 0 0 12px; display: flex; flex-direction: column; align-items: flex-start; gap: 8px; }
  .hint { margin: 0; font-size: 12px; color: var(--card-text-2); }
  .chosen { list-style: none; margin: 0; padding: 0; display: flex; flex-wrap: wrap; gap: 10px; min-height: 30px; }
  .chosen li { display: flex; align-items: center; gap: 2px; }
  .chosen .none { font-size: 12px; color: var(--card-text-2); }
  .mini { width: 18px; height: 22px; padding: 0; border: 0; border-radius: 4px; background: transparent; color: var(--card-text-2); font-size: 15px; line-height: 1; cursor: pointer; }
  .mini:hover:not(:disabled) { background: var(--panel-top); color: var(--card-text); }
  .mini:disabled { opacity: .3; cursor: default; }
  .actions { display: flex; gap: 6px; }
  .actions .btn { height: 28px; padding: 0 12px; font-size: 12px; }
  .err { margin: 0; font-size: 12px; color: var(--danger-text); }
</style>
