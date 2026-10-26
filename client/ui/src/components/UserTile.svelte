<script lang="ts">
  import Icon from './Icon.svelte'
  import { voice } from '../lib/voice.svelte'
  import { app } from '../lib/store.svelte'
  import type { RosterEntry } from '../lib/selectors'
  import { avatarColor, initial } from '../lib/avatar'
  import { volumeLabel } from '../lib/voiceui'

  let {
    entry,
    compact = false,
    onMenu,
  }: {
    entry: RosterEntry
    /** The small tile in the full view's strip. */
    compact?: boolean
    onMenu: (e: MouseEvent | KeyboardEvent, anchor: HTMLElement) => void
  } = $props()

  const isMe = $derived(entry.userId === app.state.me?.id)
  const volume = $derived(voice.localMutes[entry.userId] ? 0 : (voice.volumes[entry.userId] ?? 100))
  const bars = $derived.by(() => {
    const q = voice.quality[entry.userId]
    return q === 'excellent' ? 3 : q === 'good' ? 2 : q === 'poor' ? 1 : q === 'lost' ? 0 : 3
  })
  // the avatar button is what has focus, so it's what the menu opens from and hands focus back to
  let avatar = $state<HTMLElement>()
  // a joining entry (yours) has no menu yet
  const locked = $derived(entry.pending)

  function ask(e: MouseEvent | KeyboardEvent) {
    if (avatar && !locked) onMenu(e, avatar)
  }
  function contextMenu(e: MouseEvent) {
    e.preventDefault()
    ask(e)
  }
  function key(e: KeyboardEvent) {
    if (e.key === 'ContextMenu' || (e.key === 'F10' && e.shiftKey)) {
      e.preventDefault()
      ask(e)
    }
  }
</script>

<!-- the context menu is a pointer shortcut; the keyboard route is the avatar button's ContextMenu / Shift+F10 -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="tile" class:compact class:pending={entry.pending} class:speaking={voice.speaking.has(entry.userId)} data-user={entry.userId} oncontextmenu={contextMenu}>
  <button bind:this={avatar} type="button" class="avatar-btn" data-menu-toggle aria-disabled={locked} aria-haspopup="menu" aria-label={entry.name} data-tip={isMe ? undefined : `Options for ${entry.name}`}
    onkeydown={key} onclick={ask}>
    <span class="av" style:background={avatarColor(entry.userId)}>{initial(entry.name)}</span>
    {#if entry.deafened}<span class="badge"><Icon name="headphonesOff" size={compact ? 11 : 14} label="Deafened" /></span>
    {:else if entry.muted}<span class="badge"><Icon name="micOff" size={compact ? 11 : 14} label="Muted" /></span>{/if}
  </button>
  <div class="name">
    <span class="text">{entry.name}</span>
    {#if entry.pending}
      <span class="joining">Joining…</span><span class="spinner" role="progressbar" aria-label="Joining"></span>
    {:else}
      <span class="bars" aria-label="Connection {voice.quality[entry.userId] ?? 'good'}">
        {#each [1, 2, 3] as b}<span class="bar" class:on={b <= bars} style:height="{b * (compact ? 3 : 4)}px"></span>{/each}
      </span>
    {/if}
  </div>
  {#if !entry.pending && volume !== 100}<span class="vol">{volumeLabel(volume)}</span>{/if}
</div>

<style>
  .tile { position: relative; aspect-ratio: 16 / 9; min-width: 0; background: var(--bg-1); border-radius: 16px; border: 2px solid transparent; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 10px; }
  .tile.speaking { border-color: var(--ok); }
  .tile.pending { opacity: .6; }
  .avatar-btn { position: relative; border: 0; padding: 0; background: none; border-radius: 50%; }
  .avatar-btn[aria-disabled='true'] { cursor: default; }
  .av { width: 84px; height: 84px; border-radius: 50%; color: #fff; font-size: 34px; font-weight: 700; display: grid; place-items: center; transition: box-shadow .12s; }
  .tile.speaking .av { box-shadow: 0 0 0 4px var(--bg-1), 0 0 0 7px var(--ok); }
  .badge { position: absolute; right: -4px; bottom: -4px; width: 28px; height: 28px; border-radius: 50%; background: var(--danger); color: #fff; border: 4px solid var(--bg-1); display: grid; place-items: center; }
  .name { display: flex; align-items: center; gap: 8px; font-weight: 600; max-width: 90%; }
  .text { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .joining { font-size: 12px; font-weight: 400; color: var(--text-2); }
  .spinner { width: 12px; height: 12px; border-radius: 50%; border: 2px solid var(--text-3); border-top-color: transparent; animation: spin .8s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
  @media (prefers-reduced-motion: reduce) { .spinner { animation-duration: 2.4s; } }
  .bars { display: flex; align-items: flex-end; gap: 2px; }
  .bar { width: 3px; border-radius: 1px; background: var(--bg-4); }
  .bar.on { background: var(--ok); }
  .vol { position: absolute; top: 10px; right: 12px; font-size: 11px; color: var(--text-3); }
  .compact { gap: 4px; border-radius: 10px; }
  .compact .av { width: 36px; height: 36px; font-size: 15px; }
  .compact.speaking .av { box-shadow: 0 0 0 2px var(--bg-1), 0 0 0 4px var(--ok); }
  .compact .badge { width: 16px; height: 16px; border-width: 2px; right: -3px; bottom: -3px; }
  .compact .name { font-size: 12px; gap: 5px; }
  .compact .vol { top: 4px; right: 6px; font-size: 10px; }
</style>
