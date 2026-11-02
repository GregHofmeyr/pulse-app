<script lang="ts">
  import Segmented from './Segmented.svelte'
  import { motion, setAnimateMode, type AnimateMode } from '../lib/animate.svelte'
  import { previews, setShowPreviews } from '../lib/linkPreviews.svelte'

  const modes: { value: AnimateMode; label: string; help: string }[] = [
    { value: 'always', label: 'Always', help: 'Animated emoji play while Pulse is in focus.' },
    { value: 'hover', label: 'On hover', help: 'Animated emoji play while your pointer is over them.' },
    { value: 'never', label: 'Never', help: 'Animated emoji show their first frame.' },
  ]
  const onOff: { value: boolean; label: string }[] = [
    { value: true, label: 'On' },
    { value: false, label: 'Off' },
  ]
</script>

<section class="settings-pane">
  <h2>Chat</h2>
  <div class="settings-field"><span class="settings-label">ANIMATE EMOJI</span>
    <Segmented label="Animate emoji" options={modes} value={motion.mode} onchange={setAnimateMode} />
    <small>{modes.find((m) => m.value === motion.mode)?.help} Your system's reduce-motion setting always stops them.</small>
  </div>
  <div class="settings-field"><span class="settings-label">SHOW LINK PREVIEWS</span>
    <Segmented label="Show link previews" options={onOff} value={previews.show} onchange={setShowPreviews} />
    <small>{previews.show ? 'Links in messages show a preview card; videos and music play when you click them.' : 'Links show as plain links.'} This device only.</small>
  </div>
</section>
