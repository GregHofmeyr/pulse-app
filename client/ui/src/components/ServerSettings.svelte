<script lang="ts">
  import Icon from './Icon.svelte'
  import { modal } from '../lib/actions'
  import EmojiSettings from './EmojiSettings.svelte'
  import StorageSettings from './StorageSettings.svelte'
  import SpeakerLinkSettings from './SpeakerLinkSettings.svelte'
  import type { Server } from '../lib/protocol/Server'

  let { server, onClose }: { server: Server; onClose: () => void } = $props()
  const sections = ['Emoji', 'Quote speakers', 'Storage'] as const
  let section = $state<(typeof sections)[number]>('Emoji')
</script>

<div class="overlay modal-overlay">
<div class="backdrop modal-backdrop" role="presentation" data-backdrop onclick={onClose}></div>
<div class="modal" role="dialog" aria-modal="true" aria-label="Server settings" tabindex="-1" use:modal={onClose}>
  <nav aria-label="Server settings sections">
    <h1>{server.name}</h1>
    {#each sections as s (s)}
      <button class:on={section === s} aria-current={section === s ? 'page' : undefined} onclick={() => (section = s)}>{s}</button>
    {/each}
  </nav>
  <div class="content">
    <button class="close icon-btn round modal-close" aria-label="Close server settings" onclick={onClose}><Icon name="x" size={16} /></button>
    {#if section === 'Emoji'}<EmojiSettings {server} />{:else if section === 'Storage'}<StorageSettings {server} />{:else}<SpeakerLinkSettings {server} />{/if}
  </div>
</div>
</div>

<style>
  .overlay { position: fixed; inset: 0; z-index: 20; padding: 32px 16px; }
  /* a window like Settings: bg-1, nav and content set their own padding */
  .modal { width: min(940px, calc(100vw - 32px)); height: min(720px, calc(100vh - 64px)); flex-direction: row; gap: 0; padding: 0; background: var(--bg-1); overflow: hidden; }
  nav { width: 200px; flex-shrink: 0; padding: 24px 12px; background: var(--bg-0); display: flex; flex-direction: column; gap: 2px; }
  h1 { margin: 0 10px 12px; font-size: 12px; font-weight: 700; letter-spacing: .06em; text-transform: uppercase; color: var(--text-3); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  nav button { text-align: left; padding: 8px 10px; border: 0; border-radius: 8px; background: transparent; color: var(--text-2); font-weight: 600; }
  nav button:hover { background: var(--bg-2); color: var(--text); }
  nav button.on { background: var(--bg-3); color: var(--text); }
  .content { position: relative; flex: 1; min-width: 0; overflow-y: auto; padding: 28px 32px; }
  .close { position: absolute; top: 20px; right: 20px; z-index: 1; }
  @media (max-width: 640px) {
    .modal { flex-direction: column; }
    nav { width: auto; flex-direction: row; padding: 12px; }
    h1 { display: none; }
  }
</style>
