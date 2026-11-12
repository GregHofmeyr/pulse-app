// Inbox items for tests (#306). Not used by the app.
import type { InboxItem } from './protocol/InboxItem'
import type { Message } from './protocol/Message'

/** A plain message by SAM in C1 mentioning ALEX; `over` replaces any field. */
export function testMessage(id: string, over: Partial<Message> = {}): Message {
  return {
    id, channel_id: 'C1', author_id: 'SAM', kind: 'normal', content: 'hi', reply_to_id: null, created_at: '2026-11-12T10:00:00.000Z',
    edited_at: null, deleted: false, mentions: ['ALEX'], reactions: [], thread_root_id: null, quote: null, clip: null, embeds: [], call: null,
    qoty_card: null, pin: null, pin_line: null, wheel: null, poll: null, poll_ended: null, stage_share_line: null, gif: null, attachments: [],
    caption_is_fallback: false,
    ...over,
  } as Message
}

/** An unread mention of `testMessage(id, msg)`; `over` replaces any field. */
export function testItem(id: string, over: Partial<InboxItem> = {}, msg: Partial<Message> = {}): InboxItem {
  return { message: testMessage(id, msg), kind: 'mention', replied_to: null, read: false, ...over }
}
