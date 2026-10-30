<script lang="ts" module>
  export type Section = 'profile' | 'appearance' | 'voice' | 'keybinds' | 'chat' | 'advanced'
</script>

<script lang="ts">
  import { onMount, tick } from 'svelte'
  import Icon from './Icon.svelte'
  import { modal } from '../lib/actions'
  import VoiceSettings from './VoiceSettings.svelte'
  import KeybindSettings from './KeybindSettings.svelte'
  import ChatSettings from './ChatSettings.svelte'
  import AdvancedSettings from './AdvancedSettings.svelte'
  import AppearanceSettings from './AppearanceSettings.svelte'
  import ProfileSettings from './ProfileSettings.svelte'
  let { onClose, section = 'voice' }: { onClose: () => void; section?: Section } = $props()

  let current = $state<Section>('voice')
  onMount(() => (current = section))
  const sections: { key: Section; label: string }[] = [
    { key: 'profile', label: 'Profile' },
    { key: 'appearance', label: 'Appearance' },
    { key: 'voice', label: 'Voice & Audio' },
    { key: 'keybinds', label: 'Keybinds' },
    { key: 'chat', label: 'Chat' },
    { key: 'advanced', label: 'Advanced' },
  ]

  // Unsaved profile changes: every way out (close, backdrop, Escape, another section) asks first.
  // A Save on its way is waited for: it isn't something to discard, and may yet fail and keep the draft.
  // Meanwhile the latest way out asked for is the one taken (Profile again means stay), and it shows
  // as busy.
  type Way = { via: 'close' | Section; then: () => void }
  let profileDirty = $state(false)
  let profileSettling = $state<Promise<void> | null>(null)
  let leaving = $state<(() => void) | null>(null)
  let waiting = $state<Way | null>(null)
  // A Save puts away a discard prompt shown before it, like "Keep editing": that way out is dropped. A way out
  // asked for during the Save is taken (or asked about, if still unsaved) once it settles.
  $effect(() => {
    if (profileSettling) leaving = null
  })
  async function leave(way: Way) {
    if (current === 'profile' && profileSettling) {
      const first = !waiting
      waiting = way
      if (!first) return
      try {
        await profileSettling
        await tick() // the editor's dirty flag follows its answer
      } finally {
        way = waiting!
        waiting = null
      }
      if (way.via === 'profile') return
    }
    if (current === 'profile' && profileDirty) leaving = way.then
    else way.then()
  }
  const close = () => void leave({ via: 'close', then: onClose })
  function go(s: Section) {
    if (s !== current) void leave({ via: s, then: () => (current = s) })
    else if (waiting) waiting = { via: s, then: () => {} }
  }
  // Escape while asking means "Keep editing".
  const escape = () => (leaving ? (leaving = null) : close())
  function discard() {
    const then = leaving
    leaving = null
    profileDirty = false
    then?.()
  }
  const focusOnShow = (el: HTMLElement) => el.focus()
</script>

<div class="overlay modal-overlay">
<div class="backdrop modal-backdrop" role="presentation" data-backdrop onclick={close}></div>
<div class="modal" role="dialog" aria-modal="true" aria-label="Settings" tabindex="-1" use:modal={escape}>
  <nav aria-label="Settings sections">
    <h1>Settings</h1>
    {#each sections as s}
      <button class:on={current === s.key} aria-current={current === s.key ? 'page' : undefined} aria-busy={waiting?.via === s.key || undefined}
        onclick={() => go(s.key)}>{s.label}</button>
    {/each}
  </nav>
  <div class="content">
    <button class="close icon-btn round modal-close" aria-label="Close settings" aria-busy={waiting?.via === 'close' || undefined} onclick={close}><Icon name="x" size={16} /></button>
    {#if waiting && waiting.via !== 'profile'}
      <div class="confirm saving" role="status">Saving your profile…</div>
    {:else if leaving}
      <div class="confirm" role="alert">
        <span>Discard your profile changes?</span>
        <button type="button" class="btn btn-secondary" {@attach focusOnShow} onclick={() => (leaving = null)}>Keep editing</button>
        <button type="button" class="btn btn-danger" onclick={discard}>Discard</button>
      </div>
    {/if}
    {#if current === 'profile'}<ProfileSettings bind:dirty={profileDirty} bind:settling={profileSettling} />
    {:else if current === 'appearance'}<AppearanceSettings />
    {:else if current === 'voice'}<VoiceSettings />{:else if current === 'keybinds'}<KeybindSettings />{:else if current === 'chat'}<ChatSettings />{:else}<AdvancedSettings />{/if}
  </div>
</div>
</div>

<style>
  .overlay { position: fixed; inset: 0; z-index: 20; padding: 32px 16px; }
  /* a window, not a dialog: bg-1 (the panes and the profile footer are drawn for it), nav and content set their own padding */
  .modal { width: min(940px, calc(100vw - 32px)); height: min(720px, calc(100vh - 64px)); flex-direction: row; gap: 0; padding: 0; background: var(--bg-1); overflow: hidden; }
  nav { width: 200px; flex-shrink: 0; padding: 24px 12px; background: var(--bg-0); display: flex; flex-direction: column; gap: 2px; }
  h1 { margin: 0 10px 12px; font-size: 12px; font-weight: 700; letter-spacing: .06em; text-transform: uppercase; color: var(--text-3); }
  nav button { text-align: left; padding: 8px 10px; border: 0; border-radius: 8px; background: transparent; color: var(--text-2); font-weight: 600; }
  nav button:hover { background: var(--bg-2); color: var(--text); }
  nav button.on { background: var(--bg-3); color: var(--text); }
  /* --settings-pad-y: the profile editor's sticky footer sits flush with the bottom through it */
  .content { --settings-pad-y: 28px; position: relative; flex: 1; min-width: 0; overflow-y: auto; padding: var(--settings-pad-y) 32px; }
  .confirm { position: sticky; top: 0; z-index: 2; display: flex; align-items: center; flex-wrap: wrap; gap: 8px; margin: 0 48px 16px 0; padding: 10px 14px; border-radius: 12px; background: var(--bg-3); border: 1px solid var(--warn); font-weight: 600; }
  .confirm span { margin-right: auto; }
  .confirm.saving { border-color: var(--bg-4); color: var(--text-2); }
  /* the way out that waits for a Save */
  nav button[aria-busy='true'], .close[aria-busy='true'] { cursor: progress; opacity: .6; }
  .close { position: absolute; top: 20px; right: 20px; z-index: 1; }
  @media (max-width: 640px) {
    .modal { flex-direction: column; }
    nav { width: auto; flex-direction: row; padding: 12px; }
    h1 { display: none; }
  }
</style>
