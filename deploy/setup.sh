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

say "host basics"
hostnamectl set-hostname pulse
timedatectl set-timezone Africa/Johannesburg
apt-get update -q
DEBIAN_FRONTEND=noninteractive apt-get install -yq ca-certificates curl openssl age rclone sqlite3 unattended-upgrades

say "automatic security updates (never an automatic reboot)"
cat >/etc/apt/apt.conf.d/52pulse-unattended <<'EOF'
APT::Periodic::Update-Package-Lists "1";
APT::Periodic::Unattended-Upgrade "1";
Unattended-Upgrade::Automatic-Reboot "false";
EOF

say "docker"
command -v docker >/dev/null || curl -fsSL https://get.docker.com | sh
systemctl enable --now docker

say "pulse user and /srv/pulse"
id pulse >/dev/null 2>&1 || useradd --create-home --shell /bin/bash pulse
usermod -aG docker pulse
install -d -o pulse -g pulse -m 750 "$APP" "$APP/data" "$APP/caddy" "$APP/caddy/data" "$APP/caddy/config"
for f in compose.yml Caddyfile livekit.yaml; do install -o pulse -g pulse -m 640 "$HERE/$f" "$APP/$f"; done
install -o pulse -g pulse -m 750 "$HERE/backup.sh" "$APP/backup.sh"

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
command -v tailscale >/dev/null || curl -fsSL https://tailscale.com/install.sh | sh
if tailscale status >/dev/null 2>&1; then
  tailscale set --ssh --hostname=pulse
else
  echo "Open the login URL below and approve this machine:"
  tailscale up --ssh --hostname=pulse
fi

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
fi

say "done"
echo "Next: fill the blanks in $APP/.env (ACME_EMAIL now, backups later), then 'just deploy' from your laptop."
