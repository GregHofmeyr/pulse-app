<script lang="ts" module>
  export type PickerTab = 'emoji' | 'gifs'
  const TAB_KEY = 'pulse.pickerTab'

  /** The tab the composer's picker was last left on (this device only, #461). */
  export function savedPickerTab(): PickerTab {
    try {
      return localStorage.getItem(TAB_KEY) === 'gifs' ? 'gifs' : 'emoji'
    } catch {
      return 'emoji'
    }
  }

  function savePickerTab(t: PickerTab): void {
    try {
      localStorage.setItem(TAB_KEY, t)
    } catch {
      /* ignore */
    }
  }
</script>

<script lang="ts">
  // The composer's emoji and GIF picker in one (#461): Emoji | GIFs tabs over the picker, each tab its own search, both
  // the same size, opening on the tab last used. Where GIFs can't be sent it is the plain emoji picker.
  // In the tab row, ArrowLeft and ArrowRight switch tabs.
  import { untrack } from 'svelte'
  import EmojiPicker from './EmojiPicker.svelte'
  import GifPicker from './GifPicker.svelte'
  import Segmented from './Segmented.svelte'
  import { clickOutside } from '../lib/actions'
  import type { Std } from '../lib/emoji'
  import type { CustomEmoji } from '../lib/protocol/CustomEmoji'
  import type { Gif } from '../lib/protocol/Gif'
  import { memberServers, popoverStyle } from '../lib/reactions'
  import { app } from '../lib/store.svelte'

  let { anchor, onPick, onPickGif, onClose }: {
    /** The button that opened it; give it `data-menu-toggle` so a press on it toggles instead. */
    anchor: HTMLElement
    onPick: (e: Std | CustomEmoji, opts: { keep: boolean }) => void
    /** Sends a GIF; without it there's no GIFs tab. */
    onPickGif?: (gif: Gif, link: string) => void
    onClose: () => void
  } = $props()

  // Placed once on open: it stays put while the picker is in use.
  const style = untrack(() => popoverStyle(anchor.getBoundingClientRect(), { width: 360, height: onPickGif ? 460 : 420 }, { width: window.innerWidth, height: window.innerHeight }))
  let tab = $state<PickerTab>(untrack(() => (onPickGif ? savedPickerTab() : 'emoji')))
  function setTab(t: PickerTab) {
    tab = t
    savePickerTab(t)
  }
  const TABS: { value: PickerTab; label: string }[] = [{ value: 'emoji', label: 'Emoji' }, { value: 'gifs', label: 'GIFs' }]

  function tabKeys(e: KeyboardEvent) {
    if (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight') return
    e.preventDefault()
    e.stopPropagation() // the picker's own arrows move in its grid
    setTab(tab === 'emoji' ? 'gifs' : 'emoji')
    void Promise.resolve().then(() => document.querySelector<HTMLElement>('.emoji-popover .tabs [aria-checked="true"]')?.focus())
  }
</script>

{#snippet tabRow()}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="tab-row" onkeydown={tabKeys}>
    <Segmented label="Emoji or GIFs" options={TABS} value={tab} onchange={setTab} size="sm" fill />
  </div>
{/snippet}

<div class="emoji-popover pop-in" class:has-tabs={!!onPickGif} use:clickOutside={onClose} {style}>
  {#if tab === 'gifs' && onPickGif}
    <GifPicker onPick={onPickGif} {onClose} tabs={tabRow} />
  {:else}
    <EmojiPicker {onPick} {onClose} servers={app.state.servers} memberOf={memberServers(app.state)} tabs={onPickGif ? tabRow : undefined} />
  {/if}
</div>

<style>
  .tab-row { flex: 1; display: flex; }
  /* both tabs the same size: the tab row comes out of each picker's own height, never adds to one of them */
  .has-tabs :global(.picker) { height: 460px; }
</style>
