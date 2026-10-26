#!/usr/bin/env bash
# Turn a fresh Ubuntu 24.04 server into a Pulse host. Safe to re-run: every step checks first.
# Run as root from the folder `just provision` copies:   sudo bash setup.sh [--close-ssh]
#   --close-ssh   close public port 22 (`just close-public-ssh` passes it, over Tailscale SSH)
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
APP=/srv/pulse
CLOSE_SSH=false
[[ ${1:-} == --close-ssh ]] && CLOSE_SSH=true
[[ $EUID -eq 0 ]] || { echo "run as root (sudo)" >&2; exit 1; }
# shellcheck source=/dev/null
. /etc/os-release
[[ $ID == ubuntu ]] || { echo "Ubuntu only" >&2; exit 1; }
say() { printf '\n== %s\n' "$*"; }

# Vendor apt repositories are trusted only through a signing key whose full fingerprint is pinned here, so
# nothing from the vendors' install scripts runs as root. A key that doesn't match fails the run.
# Docker: https://docs.docker.com/engine/install/ubuntu/ (key 0EBFCD88). Tailscale: the key served at
# https://pkgs.tailscale.com/stable/ubuntu/<codename>.noarmor.gpg (identical for every codename).
DOCKER_KEY_FPR=9DC858229FC7DD38854AE2D88D81803C0EBFCD88
TAILSCALE_KEY_FPR=2596A99EAAB33821893C0A79458CA832957F5868
CODENAME=${UBUNTU_CODENAME:-$VERSION_CODENAME}
ARCH=$(dpkg --print-architecture)

# primary_fprs FILE: the fingerprint of every primary key in a (binary or armored) key file, one per line.
primary_fprs() {
  gpg --show-keys --with-colons "$1" | awk -F: '$1 == "pub" { want = 1; next } $1 == "fpr" && want { print $10; want = 0 }'
}
# install_key URL DEST FPR: fetch a signing key, require it to be exactly the pinned one, install it as a
# binary keyring at DEST. Does nothing if DEST already holds exactly that key.
install_key() {
  local url=$1 dest=$2 want=$3 tmp got
  if [[ -f $dest ]] && [[ $(primary_fprs "$dest") == "$want" ]]; then return 0; fi
  tmp=$(mktemp)
  curl -fsSL "$url" -o "$tmp"
  got=$(primary_fprs "$tmp")
  if [[ $got != "$want" ]]; then
    rm -f "$tmp"
    echo "refusing: the key at $url has fingerprint '${got:-none}', expected $want" >&2
    exit 1
  fi
  install -d -m 755 "$(dirname "$dest")"
  gpg --dearmor <"$tmp" >"$dest.new" 2>/dev/null || cp "$tmp" "$dest.new" # armored (Docker) or already binary (Tailscale)
  chmod 644 "$dest.new"
  mv "$dest.new" "$dest"
  rm -f "$tmp"
}

say "host basics"
hostnamectl set-hostname pulse
timedatectl set-timezone Africa/Johannesburg
apt-get update -q
DEBIAN_FRONTEND=noninteractive apt-get install -yq ca-certificates curl gnupg openssl age rclone sqlite3 unattended-upgrades

say "automatic security updates (never an automatic reboot)"
cat >/etc/apt/apt.conf.d/52pulse-unattended <<'EOF'
APT::Periodic::Update-Package-Lists "1";
APT::Periodic::Unattended-Upgrade "1";
Unattended-Upgrade::Automatic-Reboot "false";
EOF

say "docker"
if ! command -v docker >/dev/null; then
  install_key https://download.docker.com/linux/ubuntu/gpg /etc/apt/keyrings/docker.gpg "$DOCKER_KEY_FPR"
  echo "deb [arch=$ARCH signed-by=/etc/apt/keyrings/docker.gpg] https://download.docker.com/linux/ubuntu $CODENAME stable" \
    >/etc/apt/sources.list.d/docker.list
  apt-get update -q
  DEBIAN_FRONTEND=noninteractive apt-get install -yq docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin
fi
systemctl enable --now docker

say "pulse user and /srv/pulse"
id pulse >/dev/null 2>&1 || useradd --create-home --shell /bin/bash pulse
usermod -aG docker pulse
install -d -o pulse -g pulse -m 750 "$APP" "$APP/data" "$APP/caddy" "$APP/caddy/data" "$APP/caddy/config"
for f in compose.yml Caddyfile livekit.yaml; do install -o pulse -g pulse -m 640 "$HERE/$f" "$APP/$f"; done
for f in backup.sh deploy.sh; do install -o pulse -g pulse -m 750 "$HERE/$f" "$APP/$f"; done

say "secrets (.env)"
if [[ ! -f $APP/.env ]]; then
  install -o pulse -g pulse -m 600 "$HERE/env.example" "$APP/.env"
  sed -i "s/^LIVEKIT_SECRET=.*/LIVEKIT_SECRET=$(openssl rand -hex 32)/" "$APP/.env"
fi
sed -i "s/^PULSE_UID=.*/PULSE_UID=$(id -u pulse)/; s/^PULSE_GID=.*/PULSE_GID=$(id -g pulse)/" "$APP/.env"
chown pulse:pulse "$APP/.env"; chmod 600 "$APP/.env"

say "ssh: keys only, no root"
cat >/etc/ssh/sshd_config.d/10-pulse.conf <<'EOF'
PasswordAuthentication no
KbdInteractiveAuthentication no
PermitRootLogin no
EOF
sshd -t && systemctl reload ssh

say "firewall"
# Oracle's Ubuntu image ships iptables rules that reject all but SSH; other hosts get ufw.
oracle=false
if [[ -f /etc/iptables/rules.v4 ]] && grep -q -- '-j REJECT' /etc/iptables/rules.v4; then oracle=true; fi
open_port() { # proto port
  if $oracle; then
    iptables -C INPUT -p "$1" --dport "$2" -j ACCEPT 2>/dev/null && return 0
    local n
    n=$(iptables -L INPUT --line-numbers -n | awk '$2 == "REJECT" { print $1; exit }')
    iptables -I INPUT "${n:-1}" -p "$1" --dport "$2" -j ACCEPT
  else
    ufw allow "$2/$1" >/dev/null
  fi
}
for p in tcp:80 tcp:443 tcp:7881 udp:7882 udp:3478 udp:41641; do open_port "${p%%:*}" "${p##*:}"; done
if $oracle; then
  netfilter-persistent save >/dev/null
else
  ufw allow 22/tcp >/dev/null
  ufw default deny incoming >/dev/null
  ufw --force enable >/dev/null
fi

say "tailscale (SSH over your tailnet)"
if ! command -v tailscale >/dev/null; then
  install_key "https://pkgs.tailscale.com/stable/ubuntu/$CODENAME.noarmor.gpg" /usr/share/keyrings/tailscale-archive-keyring.gpg "$TAILSCALE_KEY_FPR"
  echo "deb [signed-by=/usr/share/keyrings/tailscale-archive-keyring.gpg] https://pkgs.tailscale.com/stable/ubuntu $CODENAME main" \
    >/etc/apt/sources.list.d/tailscale.list
  apt-get update -q
  DEBIAN_FRONTEND=noninteractive apt-get install -yq tailscale
fi
systemctl enable --now tailscaled
if tailscale status >/dev/null 2>&1; then
  tailscale set --ssh --hostname=pulse
else
  echo "Open the login URL below and approve this machine:"
  tailscale up --ssh --hostname=pulse
fi

say "UDP buffers for LiveKit media"
cat >/etc/sysctl.d/60-pulse-udp.conf <<'EOF'
# Pulse: bigger UDP buffers for LiveKit media (the default 4 MiB cap drops packets under load)
net.core.rmem_max = 16777216
net.core.wmem_max = 16777216
EOF
sysctl -q -p /etc/sysctl.d/60-pulse-udp.conf

say "nightly backup timer"
install -m 644 "$HERE/pulse-backup.service" "$HERE/pulse-backup.timer" /etc/systemd/system/
systemctl daemon-reload
systemctl enable --now pulse-backup.timer

if $CLOSE_SSH; then
  say "closing public SSH"
  tailscale status >/dev/null 2>&1 || { echo "refusing: Tailscale isn't up, you'd be locked out" >&2; exit 1; }
  if $oracle; then
    iptables -D INPUT -p tcp -m state --state NEW -m tcp --dport 22 -j ACCEPT 2>/dev/null || true
    netfilter-persistent save >/dev/null
  else
    ufw delete allow 22/tcp >/dev/null || true
  fi
  echo "Public port 22 closed. Also remove the port-22 rule from the cloud firewall (README)."
  echo "Tailscale is now the only way in: make sure key expiry is disabled for 'pulse' (README section 4)."
fi

say "done"
echo "Next: fill the blanks in $APP/.env (ACME_EMAIL and LIVEKIT_NODE_IP now, backups later), then 'just deploy' from your laptop."
