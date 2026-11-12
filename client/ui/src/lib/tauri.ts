// Typed wrappers over Tauri commands. The UI never calls the network itself.
import { invoke } from '@tauri-apps/api/core'
import type { User } from './protocol/User'
import type { Profile } from './protocol/Profile'
import type { SeenBadge } from './protocol/SeenBadge'
import type { UpdateProfileRequest } from './protocol/UpdateProfileRequest'
import type { NameArchivePage } from './protocol/NameArchivePage'
import type { BadgeCatalogue } from './protocol/BadgeCatalogue'
import type { CropRect } from './crop'
import type { Server } from './protocol/Server'
import type { Channel } from './protocol/Channel'
import type { Category } from './protocol/Category'
import type { SaveLayoutRequest } from './protocol/SaveLayoutRequest'
import type { ServerLayout } from './protocol/ServerLayout'
import type { DeletedChannel } from './protocol/DeletedChannel'
import type { DeletedServer } from './protocol/DeletedServer'
import type { Member } from './protocol/Member'
import type { Status } from './protocol/Status'
import type { StartCallResponse } from './protocol/StartCallResponse'
import type { ServerVoiceMod } from './protocol/ServerVoiceMod'
import type { Message } from './protocol/Message'
import type { InboxPage } from './protocol/InboxPage'
import type { Gif } from './protocol/Gif'
import type { AttachmentKind } from './protocol/AttachmentKind'
import type { AttachmentUrl } from './protocol/AttachmentUrl'
import type { Pin } from './protocol/Pin'
import type { PinDuration } from './protocol/PinDuration'
import type { PinnedMessage } from './protocol/PinnedMessage'
import type { StageShare } from './protocol/StageShare'
import type { Wheel } from './protocol/Wheel'
import type { CreateWheelRequest } from './protocol/CreateWheelRequest'
import type { UpdateWheelRequest } from './protocol/UpdateWheelRequest'
import type { SpinWheelRequest } from './protocol/SpinWheelRequest'
import type { QuietSpin } from './protocol/QuietSpin'
import type { CreatePollRequest } from './protocol/CreatePollRequest'
import type { Poll } from './protocol/Poll'
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
import type { SearchRequest } from './protocol/SearchRequest'
import type { SearchResults } from './protocol/SearchResults'
import type { Sound } from './protocol/Sound'
import type { NotifyLevel } from './protocol/NotifyLevel'
import type { Notice } from './notify'
import type { EmbedProvider } from './protocol/EmbedProvider'
import type { Sound as StreamSound, SoundProbe, SoundStatus, WindowRef } from './stream/sound'

/** `login` / `login_mfa`: signed in, waiting for the second step, or (second step only) the ticket is gone. */
export type LoginResult = { status: 'ready'; user: User } | { status: 'mfa_required' } | { status: 'expired' }

/** The QR code as a square module matrix, row by row (true = dark); the quiet zone is not included. */
export type QrMatrix = { size: number; modules: boolean[] }
/** Mirrors `clips::editor::ClipSource` (hand-written: it never crosses the server protocol). */
export type ClipSource = {
  id: string
  path: string
  file_name: string
  size: number
  duration_ms: number
  width: number
  height: number
  fps: number
  video_codec: string
  game_guess: string | null
  /** How the editor plays it: as is, copied into an MP4, or re-encoded small. */
  preview: 'direct' | 'remux' | 'encode'
}

/** Mirrors `clips::jobs::ClipJob` (hand-written: it never crosses the server protocol). Arrives whole as `clip://job`. */
export type ClipJob = {
  id: string
  channel_id: string
  stage: 'preparing' | 'uploading' | 'posting' | 'failed' | 'done'
  /** 0 to 1 within the stage. */
  progress: number
  /** The server's or the encoder's own words; null otherwise (the card says "Upload failed — Retry"). */
  error: string | null
  caption: string
  game_id: string | null
  kept_ms: number
  /** The local poster once made (load it with `convertFileSrc`). */
  thumb_path: string | null
  /** Set on `done` when it posted; null when it was cancelled. */
  message_id: string | null
}

/** What the editor sends: kept pieces in source milliseconds. */
export type SendClip = {
  source: string
  channelId: string
  spans: { start: number; end: number }[]
  preset: 'light' | 'sharp'
  caption: string
  gameId: string | null
}

/** Mirrors `attachments::Staged` (#57): a file queued in the composer, ready to upload. */
export type StagedFile = {
  id: string
  name: string
  size: number
  content_type: string
  kind: AttachmentKind
  width: number | null
  height: number | null
  /** A small PNG thumbnail of an image (a data URL). */
  preview: string | null
}
/** Mirrors `attachments::StageResult`. */
export type StageResult =
  | { status: 'staged'; file: StagedFile }
  /** `folder`: a folder's path, which can be zipped and sent instead. */
  | { status: 'refused'; name: string; error: string; folder: string | null }
/** `attachments://progress`: bytes of a send's files stored so far. */
export type AttachmentProgress = { nonce: string; sent: number; total: number }

/** Mirrors `clips::ffmpeg::ClipSupport`. */
export type ClipSupport = { can_send: boolean; reason: string | null }
/** The manual key (grouped in fours) and the QR for the authenticator app. */
export type TwoFactorSetup = { secret: string; qr: QrMatrix }

/** What startup found: signed in, show login, or signed in but the server refused the connection. */
/** A newer Pulse the core can install (#82). */
/** One GIF in the GIF picker (#58), as GIPHY gave it (the core's `gifs::GifResult`): `preview` and `still` are GIPHY's URLs, untouched. */
export type GifResult = { id: string; title: string; url: string; width: number; height: number; preview: string; preview_width: number; preview_height: number; still: string }
/** A page of GIF results; `next`: the next page's offset, null at the end. */
export type GifPage = { gifs: GifResult[]; next: number | null }

export type UpdateOffer = { version: string }
/** How far the installer download is (`pulse://update-progress`); `total` null = the size isn't known. */
export type UpdateProgress = { downloaded: number; total: number | null }
/** Strong (low delay)'s model, downloaded on first use (#77; also each `pulse://model` event). */
export type ModelStatus =
  | { state: 'missing' }
  | { state: 'downloading'; downloaded: number; total: number | null }
  | { state: 'ready' }
  | { state: 'failed'; error: string }
/** What Spotify plays, as this device's media session tells it (#275, the core's `activity::NowPlaying`). */
export type NowPlaying = { title: string; artist: string | null; album: string | null; spotify_id: string | null }
/** Whether this device reads Spotify, and what it last read (`pulse://media-activity`). */
export type MediaState = { reading: boolean; playing: NowPlaying | null }
export type Restored = { status: 'ready'; user: User } | { status: 'logged_out' } | { status: 'refused' }

function uploadProfileMedia(cmd: string, bytes: Uint8Array, crop: CropRect | null) {
  if (!crop) return invoke<MediaInfo>(cmd, bytes)
  return invoke<MediaInfo>(cmd, bytes, { headers: { 'x-pulse-crop': `${crop.x},${crop.y},${crop.w},${crop.h}` } })
}

export const api = {
  createDm: (userIds: string[]) => invoke<Channel>('create_dm', { userIds }),
  addMembers: (channelId: string, userIds: string[]) => invoke<void>('add_members', { channelId, userIds }),
  removeMember: (channelId: string, userId: string) => invoke<void>('remove_member', { channelId, userId }),
  /** A group's name (null = back to the members' names), or a server channel's (always one). */
  renameChannel: (channelId: string, name: string | null) => invoke<Channel>('rename_channel', { channelId, name }),
  /** A voice channel's bitrate in kbps (#75), or null for Default; everyone hears ChannelUpdated. */
  setChannelBitrate: (channelId: string, bitrate: number | null) => invoke<Channel>('set_channel_bitrate', { channelId, bitrate }),
  /** A server channel, to its server's Recently deleted; everyone hears ChannelDeleted. */
  deleteChannel: (channelId: string) => invoke<void>('delete_channel', { channelId }),
  listDeletedChannels: (serverId: string) => invoke<DeletedChannel[]>('list_deleted_channels', { serverId }),
  /** Back from Recently deleted, with everything in it; everyone hears ChannelRestored. */
  restoreChannel: (channelId: string) => invoke<Channel>('restore_channel', { channelId }),
  /** A channel in Recently deleted, for good (the server owner only). */
  purgeChannel: (channelId: string) => invoke<void>('purge_channel', { channelId }),
  /** Your server, to your Recently deleted (#184): `name` typed to confirm; everyone hears ServerDeleted. */
  deleteServer: (serverId: string, name: string) => invoke<void>('delete_server', { serverId, name }),
  /** Your own deleted servers, newest first. */
  listDeletedServers: () => invoke<DeletedServer[]>('list_deleted_servers'),
  /** Back from Recently deleted, whole; everyone hears ServerRestored. */
  restoreServer: (serverId: string) => invoke<Server>('restore_server', { serverId }),
  markRead: (channelId: string, messageId: string) => invoke<void>('mark_read', { channelId, messageId }),
  /** The channel menu's Mark as read (#111): the channel and its quote and clip reply threads. */
  markChannelRead: (channelId: string) => invoke<void>('mark_channel_read', { channelId }),
  setMute: (targetKind: 'server' | 'channel', targetId: string, until: string | null) => invoke<void>('set_mute', { targetKind, targetId, until }),
  clearMute: (targetKind: 'server' | 'channel', targetId: string) => invoke<void>('clear_mute', { targetKind, targetId }),
  /** Your notification level for a server or a server channel (#195); null = back to the default. */
  setNotifyLevel: (targetKind: 'server' | 'channel', targetId: string, level: NotifyLevel | null) =>
    invoke<void>('set_notify_level', { targetKind, targetId, level }),
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
  /** In `categoryId` (#54) at its bottom; without one, at the bottom of the uncategorised area. */
  createChannel: (serverId: string, kind: 'text' | 'quotes' | 'clips' | 'voice', name: string, categoryId?: string | null) =>
    invoke<Channel>('create_channel', categoryId ? { serverId, kind, name, categoryId } : { serverId, kind, name }),
  /** Categories (#54): everyone in the server hears LayoutChanged for each of these. */
  createCategory: (serverId: string, name: string) => invoke<Category>('create_category', { serverId, name }),
  renameCategory: (categoryId: string, name: string) => invoke<Category>('rename_category', { categoryId, name }),
  /** Its channels move to the top of the list, keeping their order. */
  deleteCategory: (categoryId: string) => invoke<void>('delete_category', { categoryId }),
  /** The server's whole order; refused when `layout.revision` isn't the server's current one. */
  saveLayout: (serverId: string, layout: SaveLayoutRequest) => invoke<ServerLayout>('save_layout', { serverId, layout }),
  joinServer: (serverId: string) => invoke<void>('join_server', { serverId }),
  listChannels: (serverId: string) => invoke<Channel[]>('list_channels', { serverId }),
  listMembers: (serverId: string) => invoke<Member[]>('list_members', { serverId }),
  listMessages: (channelId: string, before?: string) =>
    invoke<Message[]>('list_messages', { channelId, before: before ?? null }),
  /** Queued in the Rust outbox; the confirmed message arrives as a MessageCreated event with this nonce. */
  sendMessage: (channelId: string, content: string, replyToId: string | null, nonce: string, gif: Gif | null = null) =>
    invoke<void>('send_message', { channelId, content, replyToId, nonce, gif }),
  editMessage: (messageId: string, content: string) => invoke<Message>('edit_message', { messageId, content }),
  /** Files (#57): queue files by path (picked or dropped), one result per path. */
  stageAttachmentPaths: (paths: string[]) => invoke<StageResult[]>('stage_attachment_paths', { paths }),
  /** A pasted file: the bytes are the body, its name a header. */
  stageAttachmentBytes: (name: string, bytes: Uint8Array) =>
    invoke<StagedFile>('stage_attachment_bytes', bytes, { headers: { 'x-name': encodeURIComponent(name) } }),
  unstageAttachment: (id: string) => invoke<void>('unstage_attachment', { id }),
  /** "Zip and send": zip a folder and queue the zip. */
  zipAttachmentFolder: (path: string) => invoke<StagedFile>('zip_attachment_folder', { path }),
  pickAttachmentFiles: () => invoke<string[]>('pick_attachment_files'),
  /** Uploads the files, then sends; the outcome arrives on `pulse://outbox` like `sendMessage`. The same nonce again
   *  after a failure uploads only what isn't stored yet. */
  sendWithAttachments: (a: {
    channelId: string
    content: string
    replyToId: string | null
    threadRootId: string | null
    nonce: string
    staged: string[]
    tempDays: number | null
  }) => invoke<void>('send_with_attachments', a),
  cancelAttachmentSend: (nonce: string) => invoke<void>('cancel_attachment_send', { nonce }),
  attachmentUrls: (messageId: string) => invoke<AttachmentUrl[]>('attachment_urls', { messageId }),
  /** `false` when the Save dialog is cancelled. */
  saveAttachment: (messageId: string, attachmentId: string, name: string) =>
    invoke<boolean>('save_attachment', { messageId, attachmentId, name }),
  deleteMessage: (messageId: string) => invoke<void>('delete_message', { messageId }),
  /** Pins (#178): the change also arrives as MessagePinned / MessageUnpinned (and the pin's line as MessageCreated). */
  pinMessage: (messageId: string, duration: PinDuration) => invoke<Pin>('pin_message', { messageId, duration }),
  unpinMessage: (messageId: string) => invoke<void>('unpin_message', { messageId }),
  /** A chat's pins, newest pin first. */
  listPins: (channelId: string) => invoke<PinnedMessage[]>('list_pins', { channelId }),
  /** A page of your inbox (#306), newest first: older than `before`; only typed mentions with `mentions`. */
  inbox: (before: string | null, mentions: boolean) => invoke<InboxPage>('inbox', { before, mentions }),
  /** How many inbox items are unread (#306). */
  inboxUnread: () => invoke<number>('inbox_unread'),
  /** An inbox item was opened (#306); the change also arrives as InboxUpdated on every device. */
  inboxRead: (messageId: string) => invoke<void>('inbox_read', { messageId }),
  /** Mark all read (#306), up to the newest item shown (null: up to now). */
  inboxReadAll: (upto: string | null) => invoke<void>('inbox_read_all', { upto }),
  /** Share a Spotify link to a voice channel's stage (#234); you must be in its call. */
  shareToStage: (channelId: string, url: string) => invoke<StageShare>('share_to_stage', { channelId, url }),
  /** Stop a stage share for everyone (#234). */
  stopStageShare: (channelId: string, shareId: string) => invoke<void>('stop_stage_share', { channelId, shareId }),
  /** Hides one link's preview on your own message, or all of them (`url` null); the update also arrives as MessageUpdated. */
  hideEmbed: (messageId: string, url: string | null) => invoke<Message>('hide_embed', { messageId, url }),
  /** Our embed page's origin, only when signed in to the server the app's CSP allows framing; else null. */
  embedOrigin: () => invoke<string | null>('embed_origin'),
  /** Instagram plays in a capability-free pop-out window (one at a time). */
  openEmbedWindow: (provider: EmbedProvider, id: string) => invoke<void>('open_embed_window', { provider, id }),
  sendTyping: (channelId: string) => invoke<void>('send_typing', { channelId }),
  /** The raw file goes as the request body (a JSON number array would be slow at 2 MB). */
  uploadEmojiMedia: (bytes: Uint8Array) => invoke<MediaInfo>('upload_emoji_media', bytes),
  getProfile: (userId: string) => invoke<Profile>('get_profile', { userId }),
  updateProfile: (request: UpdateProfileRequest) => invoke<Profile>('update_profile', { request }),
  /** Every badge as you may see it, and how many people there are to have them. */
  getBadgeCatalogue: () => invoke<BadgeCatalogue>('get_badge_catalogue'),
  /** Up to 5 of your badges for your card, in order; empty goes back to the rarest five. */
  setBadgeShowcase: (keys: string[]) => invoke<void>('set_badge_showcase', { keys }),
  /** Your trophy case showed these [NEW] (at these tiers): they aren't now. */
  markBadgesSeen: (badges: SeenBadge[]) => invoke<void>('mark_badges_seen', { badges }),
  /** The "you're muted" detector fired (the server decides whether it counts). */
  reportMutedTalk: () => invoke<void>('report_muted_talk'),
  /** You found a hidden egg (`'logo'`). */
  reportEgg: (egg: string) => invoke<void>('report_egg', { egg }),
  /** Someone's nickname in a server (`null` resets it); anyone in the server may. */
  setNickname: (serverId: string, userId: string, nickname: string | null) => invoke<Member>('set_nickname', { serverId, userId, nickname }),
  /** Server mute / deafen someone in a server's voice (`false` undoes it; left out, unchanged). Anyone else in the server may (#140). */
  setServerVoice: (serverId: string, userId: string, change: { mute?: boolean; deafen?: boolean }) => invoke<ServerVoiceMod>('server_voice', { serverId, userId, mute: change.mute ?? null, deafen: change.deafen ?? null }),
  /** Take someone out of a server's voice call now (#140). */
  disconnectVoice: (serverId: string, userId: string) => invoke<void>('disconnect_voice', { serverId, userId }),
  /** The name only you see for someone (`null` clears it). */
  setPrivateNickname: (userId: string, nickname: string | null) => invoke<void>('set_private_nickname', { userId, nickname }),
  /** Your status (#45); `dndUntil` only with 'dnd' (null: until turned off). A StatusChanged follows to every session. */
  setStatus: (status: Status, dndUntil: string | null) => invoke<void>('set_status', { status, dndUntil }),
  /** Start a call in a DM or group, ringing the others, or join the one going on (#50: `rang` false). Then join its voice room. */
  startCall: (channelId: string) => invoke<StartCallResponse>('start_call', { channelId }),
  /** Stop a call ringing you, on all your devices; the caller isn't told (#50). */
  declineCall: (channelId: string) => invoke<void>('decline_call', { channelId }),
  /** The incoming-call ringtone on loop (#50), the one picked in Settings (#174), at a linear gain on the chosen output,
   * until `stopRingtone`. */
  startRingtone: (volume: number, output: string | null, ringtone: string) =>
    invoke<void>('start_ringtone', { volume, output, ringtone }).catch(() => {}),
  stopRingtone: () => invoke<void>('stop_ringtone').catch(() => {}),
  /** Appear offline to one person, or stop (#45). An AppearOfflineChanged follows to your sessions. */
  setAppearOffline: (userId: string, hidden: boolean) => invoke<void>('set_appear_offline', { userId, hidden }),
  /** "Read media activity" and "Show my activity" (#275): reading off runs no watcher at all. */
  setMediaActivity: (read: boolean, show: boolean) => invoke<void>('set_media_activity', { read, show }),
  /** Whether this device reads Spotify now, and what it last read. */
  mediaActivity: () => invoke<MediaState>('media_activity'),
  /** Get a notification when someone goes live, or stop (#60). A LiveNotifyChanged follows to your sessions. */
  setLiveNotify: (userId: string, on: boolean) => invoke<void>('set_live_notify', { userId, on }),
  /** A group's icon (#56): an avatar-pipeline media hash, or null for the members' faces. Any member may. */
  setGroupIcon: (channelId: string, hash: string | null) => invoke<Channel>('set_group_icon', { channelId, hash }),
  /** A page of someone's Name Archive in a server, newest first. */
  nameArchive: (serverId: string, userId: string, cursor: string | null) => invoke<NameArchivePage>('name_archive', { serverId, userId, cursor }),
  /** The raw file is the body; a crop rides in the `x-pulse-crop` header as `x,y,w,h`. */
  uploadAvatar: (bytes: Uint8Array, crop: CropRect | null) => uploadProfileMedia('upload_avatar_media', bytes, crop),
  uploadBanner: (bytes: Uint8Array, crop: CropRect | null) => uploadProfileMedia('upload_banner_media', bytes, crop),
  createEmoji: (serverId: string, name: string, mediaHash: string) =>
    invoke<CustomEmoji>('create_emoji', { serverId, name, mediaHash }),
  renameEmoji: (id: string, name: string) => invoke<CustomEmoji>('rename_emoji', { id, name }),
  deleteEmoji: (id: string) => invoke<void>('delete_emoji', { id }),
  /** App updates (#82): what the check that started with the app found (it waits up to 4 s for it); null = none. */
  updateLaunch: () => invoke<UpdateOffer | null>('update_launch'),
  /** A fresh check against the signed-in server now; null = no newer version (or signed out). */
  updateCheck: () => invoke<UpdateOffer | null>('update_check'),
  /** Download and install the offer: Pulse closes and reopens on the new version. `restart` = asked for from the
   *  title bar, which the core refuses during a call. Rejects with why, and Pulse carries on as it was. */
  updateInstall: (restart: boolean) => invoke<void>('update_install', { restart }),
  /** Where Strong (low delay)'s model is (#77). */
  modelStatus: () => invoke<ModelStatus>('model_status'),
  /** Download it from the signed-in server (`pulse://model` events on the way); the status it ended in. */
  modelDownload: () => invoke<ModelStatus>('model_download'),
  /** One line in the app log (what happened and why, never what the user typed); best effort. A failure is `warn`. */
  uiLog: (line: string, level: 'warn' | 'info' = 'warn') => invoke<void>('ui_log', { line, info: level === 'info' }).catch(() => {}),
  /** A desktop notification (lib/notify.ts decided it); best effort, a refusal only dings. */
  notifyMessage: (notice: Notice) => invoke<void>('notify_message', { notice }).catch(() => {}),
  /** Take back a desktop notification shown with `tag` (#176), where the system allows it; best effort. */
  withdrawNotification: (tag: string) => invoke<void>('withdraw_notification', { tag }).catch(() => {}),
  /** `emoji`: a Unicode emoji or `custom:<id>`. */
  addReaction: (messageId: string, emoji: string) => invoke<void>('add_reaction', { messageId, emoji }),
  removeReaction: (messageId: string, emoji: string) => invoke<void>('remove_reaction', { messageId, emoji }),
  listReactors: (messageId: string, emoji: string) => invoke<string[]>('list_reactors', { messageId, emoji }),
  /** Returns the report's short id. */
  submitReport: (text: string) => invoke<string>('submit_report', { text }),
  /** An idea (#218): only the text, no log or diagnostics. Returns its short id. */
  submitIdea: (text: string) => invoke<string>('submit_idea', { text }),
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
  /** Files dropped on a quote line (Tauri gives only their paths): the first image's bytes, read by the core with the
   *  same limits as an upload; none an image, or too big, is refused in the server's words. */
  readQuoteImage: async (paths: string[]) => new Uint8Array(await invoke<ArrayBuffer>('read_quote_image', { paths })),
  postQuote: (channelId: string, body: PostQuoteRequest) => invoke<Message>('post_quote', { channelId, body }),
  editQuote: (messageId: string, body: EditQuoteRequest) => invoke<Message>('edit_quote', { messageId, body }),
  /** Newest first; `before` pages back. */
  listReplies: (messageId: string, before?: string) => invoke<Message[]>('list_replies', { messageId, before: before ?? null }),
  /** Queued in the Rust outbox like `sendMessage` (retry, failed state); the confirmed reply arrives on `pulse://outbox` and as a MessageCreated event, with `thread_root_id` set.
   *  `replyToId`: an inline "replying to" inside the same thread. */
  sendReply: (channelId: string, rootId: string, content: string, replyToId: string | null, nonce: string, gif: Gif | null = null) =>
    invoke<void>('send_reply', { channelId, rootId, content, replyToId, nonce, gif }),
  /** GIFs (#58): a page of GIPHY results (`query` blank: trending) from `offset`; null when GIFs aren't set up on this server. */
  gifFeed: (query: string, offset = 0) => invoke<GifPage | null>('gif_feed', { query, offset }),
  /** Your favourite GIFs, newest first. */
  gifFavourites: () => invoke<Gif[]>('gif_favourites'),
  setGifFavourite: (gif: Gif, on: boolean) => invoke<void>('set_gif_favourite', { gif, on }),
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
  /** The system Save dialog, named from the caption; the original file is written by the Rust core. False when cancelled. */
  saveClip: (messageId: string, caption: string) => invoke<boolean>('save_clip', { messageId, caption }),
  /** Save image (#206): the same Save dialog for a stored image (its full file, never the still), named from the caption. False when cancelled. */
  saveImage: (hash: string, caption: string) => invoke<boolean>('save_image', { hash, caption }),
  /** Puts the (frameless) main window in or out of full screen, with a clip's element fullscreen. Resolves to whether it was fullscreen before. */
  setFullscreen: (on: boolean) => invoke<boolean>('set_fullscreen', { on }),
  /** Newest first, 30 a page; `before` pages back. */
  listClips: (channelId: string, before?: string, game?: string, author?: string) =>
    invoke<Message[]>('list_clips', { channelId, before: before ?? null, game: game ?? null, author: author ?? null }),
  clipFacets: (channelId: string) => invoke<ClipFacets>('clip_facets', { channelId }),
  setClipView: (channelId: string, view: ClipView) => invoke<void>('set_clip_view', { channelId, view }),
  clipStorage: (serverId: string) => invoke<ClipStorage>('clip_storage', { serverId }),
  /** Whether this install can send clips (Windows, ffmpeg bundled); `reason` says why not. */
  clipSupport: () => invoke<ClipSupport>('clip_support'),
  /** Probe a video file and open it as an editor source; rejects with a message to show. */
  openClip: (path: string) => invoke<ClipSource>('open_clip', { path }),
  /** The system file dialog (videos only); null when cancelled. */
  pickClipFile: () => invoke<string | null>('pick_clip_file'),
  /** The file to play (the source itself when it plays as is). Progress arrives as `clip://preview` `{ source, percent }`. */
  preparePreview: (source: string) => invoke<string>('prepare_preview', { source }),
  /** One JPEG of `tiles` keyframe tiles, 72 px tall, loadable through the asset protocol. */
  clipFilmstrip: (source: string) => invoke<{ path: string; tiles: number }>('clip_filmstrip', { source }),
  /** Linux: a loopback http address to play open source `source`'s file or preview from (WebKitGTK won't play video
   *  over asset://); null elsewhere, for any other file, or once the source is closed (which ends the address). */
  videoUrl: (source: string, path: string) => invoke<string | null>('video_url', { source, path }),
  /** Deletes the preview and filmstrip temp files. */
  closeClipSource: (source: string) => invoke<void>('close_clip_source', { source }),
  /** Start sending a clip; progress arrives as `clip://job` events. Rejects with a message to show. */
  sendClip: (r: SendClip) =>
    invoke<ClipJob>('send_clip', {
      request: {
        source: r.source,
        channel_id: r.channelId,
        spans: r.spans.map((s) => ({ start_ms: s.start, end_ms: s.end })),
        preset: r.preset,
        caption: r.caption,
        game_id: r.gameId,
      },
    }),
  retryClip: (id: string) => invoke<ClipJob>('retry_clip', { id }),
  /** Stops it and deletes everything; a last `done` event follows (no `message_id`, unless a confirm already on its way posted it). */
  cancelClip: (id: string) => invoke<void>('cancel_clip', { id }),
  /** The jobs of this session (after a restart, the ones left over, failed). */
  clipJobs: () => invoke<ClipJob[]>('clip_jobs'),
  searchQuotes: (serverId: string, q: string) => invoke<Message[]>('search_quotes', { serverId, q }),
  /** Message search in one server (`serverId`) or one DM or group (`channelId`). */
  searchMessages: (scope: { serverId: string | null; channelId: string | null }, query: SearchRequest) =>
    invoke<SearchResults>('search_messages', { ...scope, query }),
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
/** Ghost calls (watching from outside the voice channel) say so; the others look as they always have. */
const asGhost = (ghost: boolean) => (ghost ? { ghost: true } : {})

export const streamCmd = {
  ticket: (channelId: string, role: 'v' | 'a', ghost = false) => invoke<VoiceTokenResponse>('stream_ticket', { channelId, role, ...asGhost(ghost) }),
  goLive: (channelId: string, settings: StreamSettings) => invoke<StreamInfo>('stream_go_live', { channelId, settings }),
  /** Announces a stream we are still sending, after a server restart; retries while the server catches up. */
  reannounce: (channelId: string, settings: StreamSettings) => invoke<StreamInfo>('stream_reannounce', { channelId, settings }),
  stop: (channelId: string) => invoke<void>('stream_stop', { channelId }),
  /** `ghost`: counted, never named (you're not in that voice channel). */
  watch: (channelId: string, streamer: string, ghost = false) => invoke<void>('stream_watch', { channelId, streamer, ...asGhost(ghost) }),
  unwatch: (channelId: string, streamer: string) => invoke<void>('stream_unwatch', { channelId, streamer }),
  /** `imageBase64` (WebP, or JPEG/PNG where the webview can't encode WebP) without the `data:` prefix. */
  previewPut: (channelId: string, imageBase64: string) => invoke<void>('stream_preview_put', { channelId, imageBase64 }),
  /** A `data:` URL, or null when the streamer hasn't sent one yet. */
  previewGet: (channelId: string, userId: string) => invoke<string | null>('stream_preview_get', { channelId, userId }),
  /** One JSON object, no newlines; Rust prefixes `share: `. */
  log: (line: object) => invoke<void>('stream_log', { line: JSON.stringify(line) }).catch(() => {}),
  pickerWindow: (open: boolean) => invoke<void>('stream_picker_window', { open }),
  /** Publishes `muted` from the first frame (a separate mute would let a moment through). */
  /** `window`: the shared window (`null`: a screen, always everything but Pulse); `sound` where it comes from. */
  audioStart: (channelId: string, window: WindowRef | null, sound: StreamSound, muted: boolean) => invoke<void>('stream_audio_start', { channelId, window, sound, muted }),
  /** While choosing: can the window's app be told apart, and is it playing now. */
  audioProbe: (window: WindowRef) => invoke<SoundProbe>('stream_audio_probe', { window }),
  /** Switches the live stream's source without republishing. */
  audioSound: (sound: StreamSound) => invoke<void>('stream_audio_sound', { sound }),
  /** The live stream's source, `null` when nothing is published. */
  audioStatus: () => invoke<SoundStatus | null>('stream_audio_status'),
  audioStop: () => invoke<void>('stream_audio_stop'),
  /** `#a` leaves unless it is in `keep` (the voice channel now; `null` = no voice). */
  audioLeave: (keep: string | null) => invoke<void>('stream_audio_leave', { keep }),
  audioMute: (muted: boolean) => invoke<void>('stream_audio_mute', { muted }),
  /** A ghost's stream audio plays on `output` (no call to play through). */
  audioWatch: (channelId: string, streamer: string, on: boolean, ghost = false, output: string | null = null) =>
    invoke<void>('stream_audio_watch', { channelId, streamer, on, ...(ghost ? { ghost: true, output } : {}) }),
  setVolume: (userId: string, pct: number) => invoke<void>('set_stream_volume', { userId, pct }),
}

/** Soundboard commands. `output` is the chosen output device name (null = default). */
export type SoundPreview = { duration_ms: number; peaks: number[] }
export type SoundUpdate = { name?: string; emoji?: string | null; emoji_id?: string | null; volume?: number }
/** Hearing your own play to the call on click (#143): the levels its echo would play at, and you as its player. */
export type HearNow = { hash: string; callsPercent: number; soundboardPercent: number; soundPercent: number; player: string }
export const soundboardCmd = {
  support: () => invoke<{ can_send: boolean; reason: string | null }>('soundboard_support'),
  pickFile: () => invoke<string | null>('soundboard_pick_file'),
  open: (path: string) => invoke<SoundPreview>('soundboard_open', { path }),
  preview: (path: string, startMs: number, endMs: number, soundPercent: number, callsPercent: number, soundboardPercent: number, output: string | null) =>
    invoke<void>('soundboard_preview', { path, startMs, endMs, soundPercent, callsPercent, soundboardPercent, output }),
  upload: (path: string, startMs: number, endMs: number) => invoke<MediaInfo>('soundboard_upload', { path, startMs, endMs }),
  /** Local playback at Calls × Soundboard × the sound's own volume (percents; the core multiplies and caps). `player`:
   *  who played it into the call, whose previous sound it cuts off (#214); a preview has none and cuts nothing. */
  play: (hash: string, callsPercent: number, soundboardPercent: number, soundPercent: number, output: string | null, soundId: string | null = null, player: string | null = null) =>
    invoke<void>('soundboard_play', { hash, callsPercent, soundboardPercent, soundPercent, output, soundId, player }),
  /** Fetch these sounds into the disk cache ahead of their first play (joining a call). Returns at once. */
  warm: (hashes: string[]) => invoke<void>('soundboard_warm', { hashes }),
  /** The trimmer's play: the picked file from `fromMs` to `toMs`, here only. Resolves with the ms it will play. */
  trimPlay: (path: string, fromMs: number, toMs: number, callsPercent: number, soundboardPercent: number, output: string | null) =>
    invoke<number>('soundboard_trim_play', { path, fromMs, toMs, callsPercent, soundboardPercent, output }),
  trimStop: () => invoke<void>('soundboard_trim_stop'),
  create: (serverId: string, name: string, emoji: string | null, emojiId: string | null, mediaHash: string, volume: number) =>
    invoke<Sound>('create_sound', { serverId, name, emoji, emojiId, mediaHash, volume }),
  update: (id: string, request: SoundUpdate) => invoke<Sound>('update_sound', { id, request }),
  remove: (id: string) => invoke<void>('delete_sound', { id }),
  list: (serverId: string) => invoke<Sound[]>('list_sounds', { serverId }),
  /** Play to everyone in your call. `hearNow` (#143): you hear it at once rather than on its echo; the core stops it
   *  if the server refuses. Rejects with `{ message, code }` (`code: 'rate_limited'` for a 429). */
  playToCall: (id: string, hearNow: HearNow | null = null) =>
    invoke<void>('play_sound_to_call', hearNow ? { id, hearNow } : { id }),
  setFavourite: (id: string, on: boolean) => invoke<void>('set_sound_favourite', { id, on }),
  /** Your join sound in `serverId` (#121): one of its sounds, or null for the normal join cue. */
  setJoinSound: (serverId: string, soundId: string | null) => invoke<void>('set_join_sound', { serverId, soundId }),
}

/** The picker wheel (#158): saved wheels, spins, and Spin again from a spin's line. */
export const wheelCmd = {
  listServer: (serverId: string) => invoke<Wheel[]>('list_server_wheels', { serverId }),
  listMine: () => invoke<Wheel[]>('list_my_wheels'),
  /** In `serverId`, or Just me with none. */
  create: (serverId: string | null, name: string, options: string[]) =>
    invoke<Wheel>('create_wheel', { request: { ...(serverId ? { server_id: serverId } : {}), name, options } satisfies CreateWheelRequest }),
  update: (id: string, request: UpdateWheelRequest) => invoke<Wheel>('update_wheel', { id, request }),
  remove: (id: string) => invoke<void>('delete_wheel', { id }),
  /** A saved wheel by id, or a quick one from its options. Resolves with the spin's line. */
  spin: (channelId: string, request: SpinWheelRequest) => invoke<Message>('spin_wheel', { channelId, request }),
  spinAgain: (messageId: string, removeWinner: boolean) => invoke<Message>('spin_again', { messageId, removeWinner }),
  /** A spin the chat watches but nothing saves (#270). Resolves with the spin the server drew. */
  spinQuietly: (channelId: string, request: SpinWheelRequest) => invoke<QuietSpin>('spin_wheel_quietly', { channelId, request }),
}

/** Polls (#157): post one, fetch your view of one, vote (your full set of choices; none takes it back), end yours. */
export const pollCmd = {
  create: (channelId: string, request: CreatePollRequest) => invoke<Message>('create_poll', { channelId, request }),
  get: (messageId: string) => invoke<Poll>('get_poll', { messageId }),
  vote: (messageId: string, options: number[]) => invoke<Poll>('vote_poll', { messageId, options }),
  close: (messageId: string) => invoke<Poll>('close_poll', { messageId }),
}

/** Tauri rejects with the command's error string. */
export function errorText(e: unknown): string {
  if (typeof e === 'string') return e
  if (e instanceof Error) return e.message
  // a coded core error (`{ message, code }`, e.g. play_sound_to_call)
  if (typeof e === 'object' && e !== null && typeof (e as { message?: unknown }).message === 'string') return (e as { message: string }).message
  return 'Something went wrong'
}
