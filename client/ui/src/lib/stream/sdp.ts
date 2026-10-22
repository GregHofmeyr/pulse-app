/** Without rtx-time, libwebrtc gives up on a lost packet and freezes until a keyframe (~3 s at 1% loss). */
export function addRtxTime(sdp: string, ms: number): string {
  return sdp.replace(/^(a=fmtp:\d+ apt=\d+)(?!.*rtx-time)(.*)$/gm, `$1$2;rtx-time=${ms}`)
}

const RTX_TIME_MS = 300
const patched = new WeakSet<object>()

/** Patches `RTCPeerConnection.prototype.setLocalDescription` once so every answer carries rtx-time=300. */
export function installRtxTime(): void {
  const proto = (globalThis as { RTCPeerConnection?: { prototype: RTCPeerConnection } }).RTCPeerConnection?.prototype
  if (!proto || patched.has(proto)) return
  patched.add(proto)
  const orig = proto.setLocalDescription as (this: RTCPeerConnection, d?: RTCLocalSessionDescriptionInit) => Promise<void>
  proto.setLocalDescription = function (this: RTCPeerConnection, desc?: RTCLocalSessionDescriptionInit) {
    if (desc?.type === 'answer' && desc.sdp) desc = { ...desc, sdp: addRtxTime(desc.sdp, RTX_TIME_MS) }
    return orig.call(this, desc)
  }
}
