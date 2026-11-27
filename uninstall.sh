#!/bin/sh
# Take r3cui off an Unfolded Circle Remote 3: switch back to the Normal UI, or remove r3cui completely.
#
#   ./uninstall.sh                       asks for everything
#   ./uninstall.sh <IP> [PIN] [disable|remove]
#
# The warranty record written when r3cui was installed stays either way.
set -eu

ip=${1:-${R3_IP:-}}
pin=${2:-${R3_PIN:-}}
how=${3:-}

command -v curl >/dev/null 2>&1 || { echo "This needs curl. Install it and run this again." >&2; exit 1; }

[ -n "$ip" ] || { printf "Remote's IP address: "; read -r ip; }
if [ -z "$pin" ]; then
  printf "Web configurator PIN: "
  stty -echo 2>/dev/null || true
  read -r pin
  stty echo 2>/dev/null || true
  echo
fi

# The PIN goes to curl on stdin, so it never shows up in the process list.
# curl prints 000 as the code when it can't connect; its own error text is left out for the messages below.
remote() {
  printf 'user = "web-configurator:%s"\n' "$pin" | curl -s -K - "$@" || true
}

if [ -z "$how" ]; then
  cat <<'EOF'

  1) Switch back to the Normal UI (r3cui stays installed, so it is easy to turn back on)
  2) Remove r3cui completely

EOF
  printf 'Choose 1 or 2: '
  read -r pick
  case $pick in 1) how=disable ;; 2) how=remove ;; *) echo "Nothing was changed."; exit 1 ;; esac
fi

case $how in
  disable) code=$(remote -o /dev/null -w '%{http_code}' --connect-timeout 8 -X PUT "http://$ip/api/system/install/ui?enable=false") ;;
  remove) code=$(remote -o /dev/null -w '%{http_code}' --connect-timeout 8 -X DELETE "http://$ip/api/system/install/ui") ;;
  *) echo "Use disable or remove." >&2; exit 1 ;;
esac

case $code in
  2??) if [ "$how" = remove ]; then echo "r3cui is removed. The remote is back on the Normal UI."; else echo "The remote is back on the Normal UI. r3cui is still installed."; fi ;;
  000) echo "Can't reach a remote at $ip. Check the address and that this computer is on the same network." >&2; exit 1 ;;
  401) echo "The remote rejected the PIN. Check the web configurator PIN in the remote's settings." >&2; exit 1 ;;
  404) echo "There is no custom UI installed on this remote." >&2; exit 1 ;;
  *) echo "The remote answered with HTTP $code; nothing may have changed." >&2; exit 1 ;;
esac
