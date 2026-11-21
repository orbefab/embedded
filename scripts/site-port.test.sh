#!/usr/bin/env bash
# Proves with_fresh_ports retries when the port it handed out is taken before
# the server binds, and that a real test failure is not retried.
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck source=scripts/site-port.sh
source scripts/site-port.sh

work=$(mktemp -d)
holder=
cleanup() {
  if [[ -n $holder ]] && kill -0 "$holder" 2>/dev/null; then
    kill "$holder" 2>/dev/null || true
    wait "$holder" 2>/dev/null || true
  fi
  rm -rf "$work"
}
trap cleanup EXIT

node -e 'const s = require("node:net").createServer().listen(0, "127.0.0.1", () => { process.stdout.write(String(s.address().port)); setInterval(() => {}, 1e6); })' >"$work/port" &
holder=$!
for _ in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do
  [[ -s $work/port ]] && break
  sleep 0.05
done
[[ -s $work/port ]] || {
  echo "scripts/site-port.test.sh: the held port never listened" >&2
  exit 1
}
held=$(cat "$work/port")

# free_port runs in a command substitution, so a counter in the shell would not stick.
echo 0 >"$work/calls"
free_port() {
  local n
  n=$(cat "$work/calls")
  n=$((n + 1))
  echo "$n" >"$work/calls"
  if [[ $n -eq 1 ]]; then
    printf '%s\n' "$held"
  else
    node -e 'const s = require("node:net").createServer().listen(0, "127.0.0.1", () => { process.stdout.write(String(s.address().port) + "\n"); s.close(); })'
  fi
}

cat >"$work/bind.sh" <<'EOF'
#!/usr/bin/env bash
node -e '
const net = require("node:net");
const port = Number(process.env.ORBE_SITE_VITE_PORT);
const s = net.createServer();
s.on("error", (e) => {
  if (e.code === "EADDRINUSE") {
    console.error("Port " + port + " is already in use");
    process.exit(1);
  }
  console.error(e);
  process.exit(1);
});
s.listen(port, "127.0.0.1", () => {
  console.log("bound " + s.address().port);
  s.close();
});
'
EOF
chmod +x "$work/bind.sh"

with_fresh_ports ORBE_SITE_VITE_PORT -- "$work/bind.sh" >"$work/out"
grep -q "is already in use" "$work/out"
grep -q "^bound " "$work/out"
bound=$(sed -n 's/^bound //p' "$work/out")
[[ $bound != "$held" ]] || {
  echo "scripts/site-port.test.sh: retried on the same port $held" >&2
  exit 1
}
[[ $(cat "$work/calls") -ge 2 ]] || {
  echo "scripts/site-port.test.sh: did not ask for a fresh port" >&2
  exit 1
}

echo 0 >"$work/calls"
free_port() {
  local n
  n=$(cat "$work/calls")
  n=$((n + 1))
  echo "$n" >"$work/calls"
  echo 9
}
set +e
with_fresh_ports ORBE_SITE_VITE_PORT -- bash -c 'echo "1 failed"; exit 1' >"$work/fail"
code=$?
set -e
[[ $code -eq 1 && $(cat "$work/calls") -eq 1 ]] || {
  echo "scripts/site-port.test.sh: a test failure was retried (exit $code, ports $(cat "$work/calls"))" >&2
  exit 1
}

echo 0 >"$work/calls"
set +e
with_fresh_ports ORBE_SITE_VITE_PORT -- bash -c 'echo "Port 9 is already in use"; echo "1 failed"; exit 1' >"$work/both"
code=$?
set -e
[[ $code -eq 1 && $(cat "$work/calls") -eq 1 ]] || {
  echo "scripts/site-port.test.sh: a failed run that also mentions the port was retried (exit $code, ports $(cat "$work/calls"))" >&2
  exit 1
}
