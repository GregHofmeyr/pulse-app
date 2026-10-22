/** S4: without rtx-time, libwebrtc gives up on a lost packet and freezes until a keyframe (~3 s at 1% loss). */
export function addRtxTime(sdp: string, ms: number): string {
  return sdp.replace(/^(a=fmtp:\d+ apt=\d+)(?!.*rtx-time)(.*)$/gm, `$1$2;rtx-time=${ms}`)
}
