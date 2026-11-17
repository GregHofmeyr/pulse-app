<script lang="ts">
  import Segmented from './Segmented.svelte'
  import ClipDropSettings from './ClipDropSettings.svelte'
  import { motion, setAnimateMode, type AnimateMode } from '../lib/animate.svelte'
  import { previews, setShowPreviews } from '../lib/linkPreviews.svelte'
  import { planPrefs, setPlanReminders, setPlanSomething } from '../lib/plans.svelte'
  import { app } from '../lib/store.svelte'
  import { errorText } from '../lib/tauri'
  import { f1Prefs, setF1RaceWeekends } from '../lib/f1Prefs.svelte'
  import { setWatchParties, watchPrefs } from '../lib/watchParty.svelte'

  const modes: { value: AnimateMode; label: string; help: string }[] = [
    { value: 'always', label: 'Always', help: 'Animated emoji play while Pulse is in focus.' },
    { value: 'hover', label: 'On hover', help: 'Animated emoji play while your pointer is over them.' },
    { value: 'never', label: 'Never', help: 'Animated emoji show their first frame.' },
  ]
  const onOff: { value: boolean; label: string }[] = [
    { value: true, label: 'On' },
    { value: false, label: 'Off' },
  ]

  // Plan reminders (#399): kept by the server, so it follows you to every device
  let remindersError = $state('')
  async function changeReminders(on: boolean) {
    remindersError = ''
    try {
      await setPlanReminders(on)
    } catch (e) {
      remindersError = errorText(e)
    }
  }
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
  <div class="settings-field"><span class="settings-label">WATCH PARTIES</span>
    <Segmented label="Watch parties" options={onOff} value={watchPrefs.on} onchange={setWatchParties} />
    <small>{watchPrefs.on ? 'In a voice channel, a YouTube card can start a watch party: everyone in the call who joins it watches the same video at the same moment.' : 'No Start or Join watch party, and a party handed to you is passed on. The channel list still shows when one is on.'} This device only.</small>
  </div>
  <div class="settings-field"><span class="settings-label">PLAN SOMETHING</span>
    <Segmented label="Plan something" options={onOff} value={planPrefs.on} onchange={setPlanSomething} />
    <small>{planPrefs.on ? 'The Plan button and /plan post invites with RSVPs. Plans show the forecast for their place, and you get a notification an hour before one you said Going or Maybe to.' : "No Plan button, forecasts or reminder notifications. Plans others post still show, and reminders still reach your inbox (unless Plan reminders is off)."} This device only.</small>
  </div>
  <div class="settings-field"><span class="settings-label">PLAN REMINDERS</span>
    <Segmented label="Plan reminders" options={onOff} value={app.state.planReminders} onchange={(v) => void changeReminders(v)} />
    <small>{app.state.planReminders ? "An hour before a plan you said Going or Maybe to, its reminder line pings you (your inbox, and a notification while Plan something is on)." : "Plans' reminder lines don't ping you: nothing in your inbox, no notification."} Every device.</small>
    {#if remindersError}<small class="err" role="alert">{remindersError}</small>{/if}
  </div>
  <div class="settings-field"><span class="settings-label">F1 RACE WEEKENDS</span>
    <Segmented label="F1 race weekends" options={onOff} value={f1Prefs.on} onchange={setF1RaceWeekends} />
    <small>{f1Prefs.on ? "A server's F1 channel shows a countdown to the next race, and F1 cards open in full: the race weekend with podium predictions, the results and the scores. You get a notification when your predictions are scored." : 'No countdown, F1 cards fold to one line (click one to open it), and no notification when your predictions are scored (it still reaches your inbox).'} This device only.</small>
  </div>
  <!-- Clip drop (#382): its own feature (Windows only), apart from Game activity -->
  <ClipDropSettings />
</section>

<style>
  .err { color: var(--danger-text); }
</style>
