// Links to a message's files (#57): one request signs all of a message's files, and each link is reused until shortly
// before it expires (linkCache), so an image re-rendered or a video replayed can come from the HTTP cache.
import type { AttachmentUrl } from './protocol/AttachmentUrl'
import { linkCache } from './linkCache'

/** The file is gone or (a temporary send) expired: it has no link. */
export class NoLink extends Error {}

const cache = linkCache()
const asking = new Map<string, Promise<AttachmentUrl[]>>()

const key = (messageId: string, attachmentId: string) => `${messageId}/${attachmentId}`

/** The link to file `attachmentId` of message `messageId`. `fetch` signs every file of the message; the others' links
 *  are kept for when they're asked for. Everything asked for one message at once shares one request. */
export function attachmentLink(
  messageId: string,
  attachmentId: string,
  fetch: (messageId: string) => Promise<AttachmentUrl[]>,
  opts: { fresh?: boolean } = {},
): Promise<string> {
  return cache.get(
    key(messageId, attachmentId),
    async () => {
      const since = cache.era()
      let all = asking.get(messageId)
      if (!all) {
        all = fetch(messageId)
        asking.set(messageId, all)
        void all.finally(() => asking.get(messageId) === all && asking.delete(messageId)).catch(() => {})
      }
      const askedAt = Date.now()
      const urls = await all
      for (const u of urls) {
        if (u.url && u.expires_at && u.id !== attachmentId) cache.put(key(messageId, u.id), { url: u.url, expires_at: u.expires_at }, askedAt, since)
      }
      const mine = urls.find((u) => u.id === attachmentId)
      if (!mine?.url) throw new NoLink()
      return { url: mine.url, expires_at: mine.expires_at ?? '' }
    },
    opts,
  )
}

/** Sign-out or account switch. */
export function forgetAttachmentLinks(): void {
  cache.forget()
  asking.clear()
}
