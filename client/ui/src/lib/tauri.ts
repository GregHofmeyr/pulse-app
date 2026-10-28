// Typed wrappers over Tauri commands. The UI never calls the network itself.
import { invoke } from '@tauri-apps/api/core'
import type { User } from './protocol/User'
import type { Server } from './protocol/Server'
import type { Channel } from './protocol/Channel'
import type { Member } from './protocol/Member'
import type { Message } from './protocol/Message'
import type { StreamInfo } from './protocol/StreamInfo'
import type { StreamSettings } from './protocol/StreamSettings'
import type { VoiceTokenResponse } from './protocol/VoiceTokenResponse'
import type { MediaInfo } from './protocol/MediaInfo'
import type { CustomEmoji } from './protocol/CustomEmoji'
import type { TwoFactorStatus } from './protocol/TwoFactorStatus'
import type { TwoFactorEnabledResponse } from './protocol/TwoFactorEnabledResponse'
import type { RecoveryCodesResponse } from './protocol/RecoveryCodesResponse'
import type { PostQuoteRequest } from './protocol/PostQuoteRequest'
import type { EditQuoteRequest } from './protocol/EditQuoteRequest'
import type { LinkSpeakerRequest } from './protocol/LinkSpeakerRequest'
import type { StartQotyRequest } from './protocol/StartQotyRequest'
import type { QotyEvent } from './protocol/QotyEvent'
import type { Leaderboard } from './protocol/Leaderboard'
import type { Game } from './protocol/Game'
import type { EditClipRequest } from './protocol/EditClipRequest'
import type { ClipUrl } from './protocol/ClipUrl'
import type { ClipFacets } from './protocol/ClipFacets'
import type { ClipView } from './protocol/ClipView'
import type { ClipStorage } from './protocol/ClipStorage'

/** `login` / `login_mfa`: signed in, waiting for the second step, or (second step only) the ticket is gone. */
export type LoginResult = { status: 'ready'; user: User } | { status: 'mfa_required' } | { status: 'expired' }

/** The QR code as a square module matrix, row by row (true = dark); the quiet zone is not included. */
export type QrMatrix = { size: number; modules: boolean[] }
/** The manual key (grouped in fours) and the QR for the authenticator app. */
export type TwoFactorSetup = { secret: string; qr: QrMatrix }

/** What startup found: signed in, show login, or signed in but the server refused the connection. */
export type Restored = { status: 'ready'; user: User } | { status: 'logged_out' } | { status: 'refused' }

export const api = {
  createDm: (userIds: string[]) => invoke<Channel>('create_dm', { userIds }),
  addMembers: (channelId: string, userIds: string[]) => invoke<void>('add_members', { channelId, userIds }),
  removeMember: (channelId: string, userId: string) => invoke<void>('remove_member', { channelId, userId }),
  renameChannel: (channelId: string, name: string | null) => invoke<Channel>('rename_channel', { channelId, name }),
  markRead: (channelId: string, messageId: string) => invoke<void>('mark_read', { channelId, messageId }),
  setMute: (targetKind: 'server' | 'channel', targetId: string, until: string | null) => invoke<void>('set_mute', { targetKind, targetId, until }),
  clearMute: (targetKind: 'server' | 'channel', targetId: string) => invoke<void>('clear_mute', { targetKind, targetId }),
  closeConversation: (channelId: string) => invoke<void>('close_conversation', { channelId }),
  login: (serverUrl: string, username: string, password: string) =>
    invoke<LoginResult>('login', { serverUrl, username, password }),
  loginMfa: (code: string) => invoke<LoginResult>('login_mfa', { code }),
  /** Back from the code step: the core forgets the pending ticket. */
  cancelLogin: () => invoke<void>('cancel_login'),
  register: (serverUrl: string, inviteCode: string, username: string, password: string) =>
    invoke<User>('register', { serverUrl, inviteCode, username, password }),
  restoreSession: () => invoke<Restored>('restore_session'),
  logout: () => invoke<void>('logout'),
  /** Sign out every other device of this account; resolves to how many were signed out. */
  logoutOthers: () => invoke<number>('logout_others'),
  /** Every other device is signed out on success. `code` only when two-factor is on. */
  changePassword: (currentPassword: string, newPassword: string, code: string | null) =>
    invoke<void>('change_password', { currentPassword, newPassword, code }),
  probe: () => invoke<void>('gateway_probe'),
  listServers: () => invoke<Server[]>('list_servers'),
  createServer: (name: string) => invoke<Server>('create_server', { name }),
  createChannel: (serverId: string, kind: 'text' | 'quotes' | 'voice', name: string) =>
    invoke<Channel>('create_channel', { serverId, kind, name }),
  joinServer: (serverId: string) => invoke<void>('join_server', { serverId }),
  listChannels: (serverId: string) => invoke<Channel[]>('list_channels', { serverId }),
  listMembers: (serverId: string) => invoke<Member[]>('list_members', { serverId }),
  listMessages: (channelId: string, before?: string) =>
    invoke<Message[]>('list_messages', { channelId, before: before ?? null }),
  /** Queued in the Rust outbox; the confirmed message arrives as a MessageCreated event with this nonce. */
  sendMessage: (channelId: string, content: string, replyToId: string | null, nonce: string) =>
    invoke<void>('send_message', { channelId, content, replyToId, nonce }),
  editMessage: (messageId: string, content: string) => invoke<Message>('edit_message', { messageId, content }),
  deleteMessage: (messageId: string) => invoke<void>('delete_message', { messageId }),
  sendTyping: (channelId: string) => invoke<void>('send_typing', { channelId }),
  /** The raw file goes as the request body (a JSON number array would be slow at 2 MB). */
  uploadEmojiMedia: (bytes: Uint8Array) => invoke<MediaInfo>('upload_emoji_media', bytes),
  createEmoji: (serverId: string, name: string, mediaHash: string) =>
    invoke<CustomEmoji>('create_emoji', { serverId, name, mediaHash }),
  renameEmoji: (id: string, name: string) => invoke<CustomEmoji>('rename_emoji', { id, name }),
  deleteEmoji: (id: string) => invoke<void>('delete_emoji', { id }),
  /** One line in the app log (what happened and why, never what the user typed); best effort. A failure is `warn`. */
  uiLog: (line: string, level: 'warn' | 'info' = 'warn') => invoke<void>('ui_log', { line, info: level === 'info' }).catch(() => {}),
  /** `emoji`: a Unicode emoji or `custom:<id>`. */
  addReaction: (messageId: string, emoji: string) => invoke<void>('add_reaction', { messageId, emoji }),
  removeReaction: (messageId: string, emoji: string) => invoke<void>('remove_reaction', { messageId, emoji }),
  listReactors: (messageId: string, emoji: string) => invoke<string[]>('list_reactors', { messageId, emoji }),
  /** Returns the report's short id. */
  submitReport: (text: string) => invoke<string>('submit_report', { text }),
  /** Writes a .txt to Downloads; returns its path. `serverUrl` when not signed in. */
  saveReport: (text: string, serverUrl: string | null) => invoke<string>('save_report', { text, serverUrl }),
  /** "Send" on last session's held crash report; returns its short id. Fails (and keeps it) if not sent. */
  sendHeldReport: () => invoke<string>('send_held_report'),
  /** "Don't send": deletes the held crash report. */
  discardHeldReport: () => invoke<void>('discard_held_report'),
  getReportSettings: () => invoke<ReportSettings>('get_report_settings'),
  setReportSettings: (settings: ReportSettings) => invoke<ReportSettings>('set_report_settings', { settings }),
  // Quotes and Quote of the Year.
  /** `null`: no such message in that channel's list (deleted, a reply, another channel's). */
  listMessagesAround: (channelId: string, messageId: string) => invoke<Message[] | null>('list_messages_around', { channelId, messageId }),
  listMessagesAfter: (channelId: string, messageId: string) => invoke<Message[]>('list_messages_after', { channelId, messageId }),
  /** The raw file goes as the request body, as for `uploadEmojiMedia`. */
  uploadQuoteMedia: (bytes: Uint8Array) => invoke<MediaInfo>('upload_quote_media', bytes),
  postQuote: (channelId: string, body: PostQuoteRequest) => invoke<Message>('post_quote', { channelId, body }),
  editQuote: (messageId: string, body: EditQuoteRequest) => invoke<Message>('edit_quote', { messageId, body }),
  /** Newest first; `before` pages back. */
  listReplies: (messageId: string, before?: string) => invoke<Message[]>('list_replies', { messageId, before: before ?? null }),
  /** Queued in the Rust outbox like `sendMessage` (retry, failed state); the confirmed reply arrives on `pulse://outbox` and as a MessageCreated event, with `thread_root_id` set.
   *  `replyToId`: an inline "replying to" inside the same thread. */
  sendReply: (channelId: string, rootId: string, content: string, replyToId: string | null, nonce: string) =>
    invoke<void>('send_reply', { channelId, rootId, content, replyToId, nonce }),
  markRepliesSeen: (rootId: string, messageId: string) => invoke<void>('mark_replies_seen', { rootId, messageId }),
  giveAward: (lineId: string) => invoke<void>('give_award', { lineId }),
  takeAward: (lineId: string) => invoke<void>('take_award', { lineId }),
  linkSpeaker: (serverId: string, body: LinkSpeakerRequest) => invoke<void>('link_speaker', { serverId, body }),
  unlinkSpeaker: (serverId: string, nameKey: string) => invoke<void>('unlink_speaker', { serverId, nameKey }),
  /** `year` omitted = this year. */
  leaderboard: (serverId: string, year?: number) => invoke<Leaderboard>('leaderboard', { serverId, year: year ?? null }),
  // Clips (the upload itself is the send_clip command, with the editor).
  addGame: (serverId: string, name: string) => invoke<Game>('add_game', { serverId, name }),
  editClip: (messageId: string, body: EditClipRequest) => invoke<Message>('edit_clip', { messageId, body }),
  /** A link that works for an hour; ask again on every play. */
  clipUrl: (messageId: string) => invoke<ClipUrl>('clip_url', { messageId }),
  /** Newest first, 30 a page; `before` pages back. */
  listClips: (channelId: string, before?: string, game?: string, author?: string) =>
    invoke<Message[]>('list_clips', { channelId, before: before ?? null, game: game ?? null, author: author ?? null }),
  clipFacets: (channelId: string) => invoke<ClipFacets>('clip_facets', { channelId }),
  setClipView: (channelId: string, view: ClipView) => invoke<void>('set_clip_view', { channelId, view }),
  clipStorage: (serverId: string) => invoke<ClipStorage>('clip_storage', { serverId }),
  searchQuotes: (serverId: string, q: string) => invoke<Message[]>('search_quotes', { serverId, q }),
  startQoty: (serverId: string, body: StartQotyRequest) => invoke<QotyEvent>('start_qoty', { serverId, body }),
  voteQoty: (eventId: string, messageId: string) => invoke<QotyEvent>('vote_qoty', { eventId, messageId }),
  unvoteQoty: (eventId: string) => invoke<QotyEvent>('unvote_qoty', { eventId }),
  addQotyOption: (eventId: string, messageId: string) => invoke<QotyEvent>('add_qoty_option', { eventId, messageId }),
  endQoty: (eventId: string) => invoke<QotyEvent>('end_qoty', { eventId }),
  /** `minutes` omitted = open until ended. */
  reopenQoty: (eventId: string, minutes?: number) => invoke<QotyEvent>('reopen_qoty', { eventId, minutes: minutes ?? null }),
}

export const twoFactor = {
  status: () => invoke<TwoFactorStatus>('two_factor_status'),
  setup: (password: string) => invoke<TwoFactorSetup>('two_factor_setup', { password }),
  enable: (code: string) => invoke<TwoFactorEnabledResponse>('two_factor_enable', { code }),
  disable: (code: string) => invoke<void>('two_factor_disable', { code }),
  regenerate: (code: string) => invoke<RecoveryCodesResponse>('two_factor_regenerate', { code }),
}

/** Off: crash reports are saved on this computer and only sent if the user says so on the next launch. */
export type ReportSettings = { auto_send_crashes: boolean }

/** Screen-share commands (the `#v` ticket, live/watch bookkeeping, previews, and the Rust `#a` audio side). */
export const streamCmd = {
  ticket: (channelId: string, role: 'v' | 'a') => invoke<VoiceTokenResponse>('stream_ticket', { channelId, role }),
  goLive: (channelId: string, settings: StreamSettings) => invoke<StreamInfo>('stream_go_live', { channelId, settings }),
  /** Announces a stream we are still sending, after a server restart; retries while the server catches up. */
  reannounce: (channelId: string, settings: StreamSettings) => invoke<StreamInfo>('stream_reannounce', { channelId, settings }),
  stop: (channelId: string) => invoke<void>('stream_stop', { channelId }),
  watch: (channelId: string, streamer: string) => invoke<void>('stream_watch', { channelId, streamer }),
  unwatch: (channelId: string, streamer: string) => invoke<void>('stream_unwatch', { channelId, streamer }),
  /** `imageBase64` (WebP, or JPEG/PNG where the webview can't encode WebP) without the `data:` prefix. */
  previewPut: (channelId: string, imageBase64: string) => invoke<void>('stream_preview_put', { channelId, imageBase64 }),
  /** A `data:` URL, or null when the streamer hasn't sent one yet. */
  previewGet: (channelId: string, userId: string) => invoke<string | null>('stream_preview_get', { channelId, userId }),
  /** One JSON object, no newlines; Rust prefixes `share: `. */
  log: (line: object) => invoke<void>('stream_log', { line: JSON.stringify(line) }).catch(() => {}),
  pickerWindow: (open: boolean) => invoke<void>('stream_picker_window', { open }),
  /** Publishes `muted` from the first frame (a separate mute would let a moment through). */
  audioStart: (channelId: string, windowTitle: string | null, muted: boolean) => invoke<void>('stream_audio_start', { channelId, windowTitle, muted }),
  audioStop: () => invoke<void>('stream_audio_stop'),
  /** `#a` leaves unless it is in `keep` (the voice channel now; `null` = no voice). */
  audioLeave: (keep: string | null) => invoke<void>('stream_audio_leave', { keep }),
  audioMute: (muted: boolean) => invoke<void>('stream_audio_mute', { muted }),
  audioWatch: (channelId: string, streamer: string, on: boolean) => invoke<void>('stream_audio_watch', { channelId, streamer, on }),
  setVolume: (userId: string, pct: number) => invoke<void>('set_stream_volume', { userId, pct }),
}

/** Tauri rejects with the command's error string. */
export function errorText(e: unknown): string {
  return typeof e === 'string' ? e : e instanceof Error ? e.message : 'Something went wrong'
}
