// The shared entrances (app.css .pop-in / .fade-in) are switched off while streaming or unfocused, so one that appears
// then isn't held invisible on its first frame. Switching one back on restarts it, though: an element that had long
// finished fading in faded in again from transparent each time the window got focus back. A native colour picker is
// its own window, so picking a colour in Settings blurred and refocused the app, and the full-screen overlay and the
// dialog flashed (#156 follow-up). An entrance that has run to its end is marked `entered`, and the off switch leaves
// marked ones alone (app.css), so focus coming and going never replays them.

const ENTRANCES = new Set(['pop-in', 'fade-in'])

/** Marks each element whose entrance finished; returns the way to stop. */
export function markFinishedEntrances(doc: Document = document): () => void {
  const ended = (e: Event) => {
    const name = (e as AnimationEvent).animationName
    if (ENTRANCES.has(name) && e.target instanceof Element) e.target.classList.add('entered')
  }
  doc.addEventListener('animationend', ended)
  return () => doc.removeEventListener('animationend', ended)
}
