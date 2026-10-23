<script lang="ts">
  import { motion, setAnimateMode, type AnimateMode } from '../lib/animate.svelte'

  const modes: { value: AnimateMode; label: string; help: string }[] = [
    { value: 'always', label: 'Always', help: 'Animated emoji play while Pulse is in focus.' },
    { value: 'hover', label: 'On hover', help: 'Animated emoji play while your pointer is over them.' },
    { value: 'never', label: 'Never', help: 'Animated emoji show their first frame.' },
  ]
</script>

<section class="pane">
  <h2>Chat</h2>
  <div class="field"><span class="lbl">ANIMATE EMOJI</span>
    <div class="seg" role="radiogroup" aria-label="Animate emoji">
      {#each modes as m}
        <button role="radio" aria-checked={motion.mode === m.value} class:on={motion.mode === m.value}
          onclick={() => setAnimateMode(m.value)}>{m.label}</button>
      {/each}
    </div>
    <small>{modes.find((m) => m.value === motion.mode)?.help} Your system's reduce-motion setting always stops them.</small>
  </div>
</section>

<style>
  .pane { display: flex; flex-direction: column; gap: 20px; }
  h2 { margin: 0; font-size: 20px; font-weight: 600; }
  .field { display: flex; flex-direction: column; gap: 8px; }
  .lbl { font-size: 12px; font-weight: 600; letter-spacing: .04em; color: var(--text-2); }
  small { font-size: 12px; color: var(--text-3); }
  .seg { display: inline-flex; align-self: flex-start; border: 1px solid var(--bg-4); border-radius: 10px; overflow: hidden; }
  .seg button { padding: 8px 14px; border: 0; background: transparent; color: var(--text-2); font-weight: 600; }
  .seg button.on { background: var(--accent); color: var(--on-accent); }
</style>
