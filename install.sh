#!/bin/sh
# Install r3cui on an Unfolded Circle Remote 3, from a computer on the same network.
#
#   ./install.sh                         asks for everything
#   ./install.sh <IP> [PIN] [file.tar.gz] uses a local archive instead of the latest release
#
# Installing a custom UI voids the remote's warranty (the remote keeps a permanent record of it).
set -eu

REPO=jackhric/r3cui
URL="https://github.com/$REPO/releases/latest/download/remote-ui.tar.gz"
ip=${1:-${R3_IP:-}}
pin=${2:-${R3_PIN:-}}
file=${3:-}

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

echo "Checking the remote at $ip..."
code=$(remote -o /dev/null -w '%{http_code}' --connect-timeout 8 "http://$ip/api/system/install/ui")
case $code in
  000) echo "Can't reach a remote at $ip. Check the address and that this computer is on the same network." >&2; exit 1 ;;
  401) echo "The remote rejected the PIN. Check the web configurator PIN in the remote's settings." >&2; exit 1 ;;
esac

cat <<'EOF'

  Installing r3cui voids your Remote 3 warranty. The remote keeps a
  permanent record that a custom UI was installed. That record stays
  even if you go back to the Normal UI later.

EOF
printf 'Type "yes" to install r3cui: '
read -r answer
[ "$answer" = yes ] || { echo "Nothing was installed."; exit 1; }

if [ -z "$file" ]; then
  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT
  file="$tmp/remote-ui.tar.gz"
  echo "Downloading the latest r3cui release..."
  curl -fL --progress-bar -o "$file" "$URL" || { echo "The download failed. Try again, or pass a remote-ui.tar.gz you downloaded." >&2; exit 1; }
fi
[ -f "$file" ] || { echo "No file at $file." >&2; exit 1; }

echo "Installing. This can take a minute..."
code=$(remote -o /dev/null -w '%{http_code}' "http://$ip/api/system/install/ui?void_warranty=yes" --form "file=@$file")
case $code in
  2??) echo "Installed. The remote switches to r3cui on its own; if it doesn't, restart the remote." ;;
  400) echo "The remote says the file is not a valid r3cui package." >&2; exit 1 ;;
  401) echo "The remote rejected the PIN." >&2; exit 1 ;;
  403) echo "The remote refused the install (it needs the warranty acknowledgement and an admin PIN)." >&2; exit 1 ;;
  409) echo "This r3cui needs newer remote firmware. Update the remote and try again." >&2; exit 1 ;;
  507) echo "The remote is out of storage." >&2; exit 1 ;;
  000) echo "Lost the connection to the remote during the install." >&2; exit 1 ;;
  *) echo "The remote answered with HTTP $code; nothing may have been installed." >&2; exit 1 ;;
esac
echo "To go back to the Normal UI, run ./uninstall.sh."
