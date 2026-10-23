<script lang="ts">
  import Segmented from './Segmented.svelte'
  import { motion, setAnimateMode, type AnimateMode } from '../lib/animate.svelte'

  const modes: { value: AnimateMode; label: string; help: string }[] = [
    { value: 'always', label: 'Always', help: 'Animated emoji play while Pulse is in focus.' },
    { value: 'hover', label: 'On hover', help: 'Animated emoji play while your pointer is over them.' },
    { value: 'never', label: 'Never', help: 'Animated emoji show their first frame.' },
  ]
</script>

<section class="settings-pane">
  <h2>Chat</h2>
  <div class="settings-field"><span class="settings-label">ANIMATE EMOJI</span>
    <Segmented label="Animate emoji" options={modes} value={motion.mode} onchange={setAnimateMode} />
    <small>{modes.find((m) => m.value === motion.mode)?.help} Your system's reduce-motion setting always stops them.</small>
  </div>
</section>
