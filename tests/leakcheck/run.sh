#!/usr/bin/env bash
# Leak check for `zcrowd sync` over Tor, the default. Needs gcc, a built zcrowd and real network access, so it is
# not part of `cargo test`. Usage: tests/leakcheck/run.sh [path/to/zcrowd]   (default: target/debug/zcrowd)
#  1. Calibration: a direct sync under the shim MUST log the server name and IP, else the shim sees nothing.
#  2. A default sync (Tor, cold Tor dir) under the shim must log no lookup of the server, no connection to its IPs and
#     no port-53 traffic. Only counts are printed, never the raw log.
#  3. Its output must be byte-identical to the direct sync of the same range, and the Tor directory must hold only
#     arti's own files with no mention of the server.
# The range is one day (the shortest `sync` accepts). Fetching its ~2000 transactions over Tor takes ~10 minutes, so the
# Tor leg starts with the direct leg's transaction cache minus 20 files: it still asks for the tip, all the blocks and
# 20 transactions, every kind of request, but the byte-identity in step 3 then proves the transactions only for those 20.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
bin=$(realpath "${1:-$here/../../target/debug/zcrowd}")
hosts="zec.rocks na.zec.rocks eu.zec.rocks"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

cc -shared -fPIC -O1 -o "$work/leakcheck.so" "$here/leakcheck.c" -ldl
getent ahosts $hosts | awk '{print $1}' | sort -u >"$work/ips"
[ -s "$work/ips" ] || { echo "cannot resolve $hosts, so there are no server IPs to check against"; exit 2; }

sync() { # sync <name> <zcrowd sync flags...>: a one-day range, under the shim
  local name=$1
  shift
  LD_PRELOAD="$work/leakcheck.so" LEAKCHECK_LOG="$work/$name.log" "$bin" sync --server https://zec.rocks:443 \
    --days 1 --out "$work/$name/flows.ndjson" "$@" >"$work/$name.out" 2>&1
}
calls='^(connect|sendto|sendmsg|sendmmsg)$'
lookups() { awk '$1 ~ /^(getaddrinfo|gethostbyname)$/' "$work/$1.log" | wc -l; }
server_lookups() { awk '$1 ~ /^(getaddrinfo|gethostbyname)$/ && $2 ~ /(^|\.)zec\.rocks$/' "$work/$1.log" | wc -l; }
server_ips() { awk -v c="$calls" 'NR==FNR {ip[$1]; next} $1 ~ c && ($2 in ip)' "$work/ips" "$work/$1.log" | wc -l; }
port53() { awk -v c="$calls" '$1 ~ c && $3 == 53' "$work/$1.log" | wc -l; }
dests() { awk '$1 == "connect" && $2 != "unix" {print $2}' "$work/$1.log" | sort -u | wc -l; }

sync direct --no-tor || { echo "direct sync failed:"; tail -5 "$work/direct.out"; exit 2; }
d_look=$(server_lookups direct) d_ip=$(server_ips direct)
echo "calibration (direct sync): lookups of the server name=$d_look, connects to server IPs=$d_ip, distinct connect destinations=$(dests direct)"
if [ "$d_look" -lt 1 ] || [ "$d_ip" -lt 1 ]; then echo "LEAKCHECK BROKEN: the shim did not see the direct sync"; exit 2; fi

end=$(sed -n 's/^blocks=.* range=[0-9]*\.\.\([0-9]*\) .*/\1/p' "$work/direct.out")
[ -n "$end" ] || { echo "no range line in the direct sync output"; exit 2; }
cached=("$work"/direct/tx/*)
[ "${#cached[@]}" -gt 20 ] || { echo "the direct sync cached ${#cached[@]} transactions, too few to leave 20 to fetch over Tor"; exit 2; }
mkdir -p "$work/tor/tx"
printf '%s\0' "${cached[@]:20}" | xargs -0 cp -t "$work/tor/tx"
sync tor --tor-dir "$work/tordir" --end-height "$end" || { echo "Tor sync failed:"; tail -5 "$work/tor.out"; exit 1; }
fetched=$(sed -n 's/^events=.* fetched_txs=\([0-9]*\) .*/\1/p' "$work/tor.out")
[ "${fetched:-0}" -ge 1 ] || { echo "the Tor sync fetched no transaction, so it did not use every kind of request"; exit 2; }
t_all=$(lookups tor) t_look=$(server_lookups tor) t_ip=$(server_ips tor) t_53=$(port53 tor)
echo "Tor sync: lookups (any name)=$t_all, lookups of the server name=$t_look, connects to server IPs=$t_ip, port-53 sends=$t_53, distinct connect destinations (Tor relays)=$(dests tor); through Tor: the tip, every block and $fetched transactions"
if [ "$t_look" -ne 0 ] || [ "$t_ip" -ne 0 ] || [ "$t_53" -ne 0 ]; then echo "LEAKCHECK FAILED"; exit 1; fi

# Same range, same bytes: Tor changes the route, not the data.
for f in flows.ndjson flows.meta.json; do
  cmp -s "$work/direct/$f" "$work/tor/$f" || { echo "OUTPUT DIFFERS from the direct sync: $f"; exit 1; }
done
echo "output through Tor is byte-identical to the direct sync ($(wc -l <"$work/tor/flows.ndjson") events; the blocks and $fetched transactions came through Tor)"

# The Tor state directory holds arti's directory data, guard state and locks, and no trace of the server.
odd=$(cd "$work/tordir" && find . -type f | grep -Ev '^\./arti-cache/(dir\.sqlite3(-wal|-shm)?|dir\.lock|dir_blobs/[^/]+)$|^\./arti-data/state/(guards\.json|circuit_timeouts\.json|state\.lock)$' || true)
[ -z "$odd" ] || { echo "unexpected files in the Tor directory:"; echo "$odd" | head -5; exit 1; }
mentions=$( (grep -rlaF -e zec.rocks "$work/tordir" || true) | wc -l)
[ "$mentions" -eq 0 ] || { echo "the Tor directory mentions the server name in $mentions file(s)"; exit 1; }
echo "Tor directory: $(find "$work/tordir" -type f | wc -l) files, all arti directory data, guard state or locks; $mentions mention the server name"
echo "LEAKCHECK OK"
