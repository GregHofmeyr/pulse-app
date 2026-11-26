<script lang="ts">
  // The typing line under a chat (#652). Its height is always held, so the list and the composer never move. A line
  // appearing from nothing rises in; changed words (one person's own text giving way to "are typing…", a name
  // changing) crossfade, the old line fading out over the new one in the same place; the last one fades out.
  import { exitFade } from '../lib/exit'
  import { swapTracker, type TypingLine } from '../lib/typingLine'

  let { line }: { line: TypingLine | null } = $props()

  const track = swapTracker()
  const swap = $derived(track(line?.key ?? null))
  // read on every change, not only while a line shows, so the tracker also sees the line go (and the next one rises)
  $effect.pre(() => void swap)

  /** How a line comes in, fixed when it's made: a repeat Typing (the same words) must not restart its entrance. Set
   *  here, not with class:, so it never changes afterwards (hence the :global classes below). */
  function enter(node: HTMLElement, swapped: boolean) {
    node.classList.add(swapped ? 'swap' : 'rise')
  }
</script>

<div class="typing" aria-live="polite">
  <!-- a keyed one-item list, so the outro is local: a line that goes (or gives way to new words) fades, but a chat
       being destroyed around it (a channel switch, closing the chat) never waits for it -->
  {#each line ? [line] : [] as l (l.key)}
    <span class="typing-line" use:enter={swap} out:exitFade={{ fadeWhenReduced: true }}><strong class="name-clip">{l.names}</strong> {l.rest}</span>
  {/each}
</div>

<style>
  .typing { position: relative; height: 22px; overflow: hidden; font-size: 12px; color: var(--text-3); }
  /* every line in the same place, so an outgoing one fades over the incoming one */
  .typing-line { position: absolute; inset: 0 18px; line-height: 22px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; will-change: opacity; }
  .typing-line strong { color: var(--text); }
  .typing-line:global(.rise) { animation: typing-rise var(--dur-slow) var(--ease-overshoot); }
  .typing-line:global(.swap) { animation: fade-in var(--dur) var(--ease); }
  /* global: lib/entrances.ts marks it finished by name */
  @keyframes -global-typing-rise { from { opacity: 0; transform: translateY(4px); } }
  /* off, not paused (app.css pauses everything while live or in the background): paused, a new line would sit on its
     invisible first frame. One that finished (.entered, lib/entrances.ts) isn't replayed when focus comes back. */
  :global(.streaming) .typing-line:not(:global(.entered)), :global(.unfocused) .typing-line:not(:global(.entered)) { animation: none !important; }
  @media (prefers-reduced-motion: reduce) {
    /* nothing moves, but it still fades: this beats app.css's global .01ms (more specific) */
    .typing-line:global(.rise), .typing-line:global(.swap) { animation: fade-in var(--dur) var(--ease) !important; }
  }
</style>
