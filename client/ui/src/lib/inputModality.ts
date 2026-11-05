// Whether the last thing the user did was press with a pointer or press a key: what opened a view
// tells it whether to take the focus (a click on a channel puts you in its message box; arrows and
// Enter in a list leave you in the list).
let last: 'pointer' | 'key' | null = null

if (typeof window !== 'undefined') {
  // capture: a handler that stops the event further down still leaves the mark
  window.addEventListener('pointerdown', () => (last = 'pointer'), true)
  window.addEventListener('keydown', () => (last = 'key'), true)
}

export const lastInput = () => last
