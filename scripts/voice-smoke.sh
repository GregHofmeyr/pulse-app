#!/usr/bin/env bash
# Proves a REAL LiveKit webhook reaches the server and records a voice session.
set -euo pipefail
cd "$(dirname "$0")/.."
TMP=$(mktemp -d); trap 'kill ${SRV:-0} 2>/dev/null || true; rm -rf "$TMP"' EXIT
export PULSE_DB_URL="sqlite://$TMP/smoke.db" PULSE_BIND=127.0.0.1:7890 PULSE_LIVEKIT_URL=ws://127.0.0.1:7880 \
       PULSE_LIVEKIT_KEY=devkey PULSE_LIVEKIT_SECRET=pulse-dev-secret-0123456789abcdefghij
docker compose up -d livekit >/dev/null
cargo build -q -p pulse-server
./target/debug/pulse-app-server serve >"$TMP/server.log" 2>&1 & SRV=$!
for _ in $(seq 50); do curl -sf localhost:7890/health >/dev/null && break; sleep 0.1; done
CODE=$(./target/debug/pulse-app-server create-invite)
J() { python3 -c "import json,sys; print(json.load(sys.stdin)$1)"; }
REG=$(curl -sf localhost:7890/auth/register -H 'content-type: application/json' \
  -d "{\"invite_code\":\"$CODE\",\"username\":\"smoke\",\"password\":\"smokesmoke\"}")
TOKEN=$(echo "$REG" | J "['token']"); UID_=$(echo "$REG" | J "['user']['id']")
SID=$(curl -sf localhost:7890/servers -H "authorization: Bearer $TOKEN" -H 'content-type: application/json' -d '{"name":"Smoke"}' | J "['id']")
LOUNGE=$(curl -sf "localhost:7890/servers/$SID/channels" -H "authorization: Bearer $TOKEN" | python3 -c "import json,sys; print([c['id'] for c in json.load(sys.stdin) if c['kind']=='voice'][0])")
docker run --rm --network host -v "$PWD/spikes/voice:/w" livekit/livekit-cli:v2.18 room join \
  --url ws://127.0.0.1:7880 --api-key devkey --api-secret "$PULSE_LIVEKIT_SECRET" \
  --identity "$UID_" --publish /w/tone.ogg --exit-after-publish "$LOUNGE" >/dev/null 2>&1
sleep 1
N=$(sqlite3 "$TMP/smoke.db" "select count(*) from voice_sessions")
echo "voice_sessions rows: $N"; [ "$N" -ge 1 ] || { echo "FAIL"; tail -20 "$TMP/server.log"; exit 1; }
echo "PASS: real LiveKit webhook recorded a voice session"
