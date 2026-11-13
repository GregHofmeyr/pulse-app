// "Message" in a person's menu (#135) where no surface hands one down: opens (or makes) your DM with them and goes
// there, as a message author's menu does; a failure shows as the app's notice.
import { openDm, ui } from './store.svelte'
import { errorText } from './tauri'

export function messagePerson(userId: string): void {
  openDm(userId).then(
    (channelId) => (ui.goTo = channelId),
    (e: unknown) => (ui.notice = errorText(e)),
  )
}
