<script lang="ts">
  // "Where you show up" (#727): a small slice of the real app under the card, as wide as the card: the voice channel
  // list on the left (you with your ring lit) and a short chat on the right (ending on your typing line). It reads
  // the editor's draft, so a name that would clip shows clipped here first. Static: the ring is a list ring and the
  // typing dots don't move. No labels, no boxes.
  import Avatar from '../Avatar.svelte'
  import Icon from '../Icon.svelte'
  import Name from '../Name.svelte'
  import SpeakingRing from '../SpeakingRing.svelte'
  import { previewUser } from '../../lib/profileDraft'
  import type { ProfileEditorState } from '../../lib/profileEditor.svelte'
  import { app } from '../../lib/store.svelte'
  import { typingLine } from '../../lib/typingLine'
  import type { User } from '../../lib/protocol/User'

  let { state: st }: { state: ProfileEditorState } = $props()

  // the other person in the slice: a test person, not anyone real
  const alex: User = { id: 'alex-preview', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
  const RING_D = 24
  const me = $derived(app.state.me)
  const pu = $derived(me ? previewUser(me, st.draft) : null)
  const line = $derived(pu ? typingLine([{ id: pu.id, name: pu.display_name, custom: pu.typing_text ?? null }]) : null)
</script>

{#if me && pu}
  <div class="show-up">
    <div class="voice" role="group" aria-label="Voice channel">
      <span class="head">VOICE</span>
      <span class="chan"><Icon name="speaker" size={14} /><span class="clip">Lounge</span></span>
      <span class="member" data-you>
        <span class="av"><Avatar id={me.id} name={pu.display_name} user={pu} size={RING_D} /><SpeakingRing userId={me.id} d={RING_D} speaking place="list" own user={pu} /></span>
        <span class="clip"><Name userId={me.id} serverId={null} user={pu} text={pu.display_name} interactive={false} surface="side" /></span>
      </span>
      <span class="member">
        <span class="av"><Avatar id={alex.id} name={alex.display_name} user={alex} size={RING_D} /></span>
        <span class="clip"><Name userId={alex.id} serverId={null} user={alex} text={alex.display_name} interactive={false} surface="side" /></span>
      </span>
    </div>
    <div class="chat" role="group" aria-label="Chat">
      <div class="msg" data-line="alex">
        <Avatar id={alex.id} name={alex.display_name} user={alex} size={28} />
        <span class="said"><span class="clip"><Name userId={alex.id} serverId={null} user={alex} text={alex.display_name} interactive={false} /></span><span class="clip text">new banner??</span></span>
      </div>
      <div class="msg" data-line="you">
        <Avatar id={me.id} name={pu.display_name} user={pu} size={28} />
        <span class="said"><span class="clip"><Name userId={me.id} serverId={null} user={pu} text={pu.display_name} interactive={false} /></span><span class="clip text">Looking good!</span></span>
      </div>
      {#if line}
        <p class="typing" data-typing>
          <span class="clip"><strong><Name userId={me.id} serverId={null} user={pu} text={pu.display_name} interactive={false} flow /></strong> {line.rest}</span>
          <span class="dots" aria-hidden="true"><i></i><i></i><i></i></span>
        </p>
      {/if}
    </div>
  </div>
{/if}

<style>
  /* as wide as the card (ProfileCard: 300px x --k), and never wider than the column it sits in */
  .show-up { --k: 1.12; box-sizing: border-box; width: min(100%, calc(300px * var(--k))); min-width: 0; display: grid; grid-template-columns: minmax(0, 1fr) minmax(0, 2fr); gap: 16px; color: var(--text); font-size: 13px; }
  .voice, .chat { min-width: 0; display: flex; flex-direction: column; gap: 6px; overflow: clip; overflow-clip-margin: .6em; }
  .head { font-size: 11px; font-weight: 700; letter-spacing: .06em; color: var(--text-3); }
  .chan { display: flex; align-items: center; gap: 4px; min-width: 0; color: var(--text-2); }
  .member { display: flex; align-items: center; gap: 6px; min-width: 0; padding-left: 8px; font-weight: 600; }
  .av { position: relative; display: inline-flex; flex-shrink: 0; }
  /* clip, not hidden: a name's glow shows a little past the box on every side, as in the app; the ellipsis still
     cuts at the box */
  .clip { min-width: 0; overflow: clip; overflow-clip-margin: .6em; padding-block: .2em; margin-block: -.2em; text-overflow: ellipsis; white-space: nowrap; }
  .msg { display: flex; align-items: flex-start; gap: 8px; min-width: 0; }
  .said { min-width: 0; flex: 1; display: flex; flex-direction: column; font-weight: 600; }
  .text { font-weight: 400; color: var(--text); }
  .typing { margin: 0; min-width: 0; display: flex; align-items: center; gap: 6px; font-size: 12px; color: var(--text-3); }
  .typing strong { color: var(--text); }
  .dots { display: inline-flex; gap: 3px; flex-shrink: 0; }
  .dots i { width: 4px; height: 4px; border-radius: 50%; background: var(--text-3); }
</style>
