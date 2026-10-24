#!/usr/bin/env bash
# Leak check for `zcrowd sync` over Tor, the default (spec 41 C7). Needs gcc, a built zcrowd and real network access, so it is
# not part of `cargo test`. Usage: tests/leakcheck/run.sh [path/to/zcrowd]   (default: target/debug/zcrowd)
#  1. Calibration: a direct sync under the shim MUST log the server name and IP, else the shim sees nothing.
#  2. A default sync (Tor, cold Tor dir) under the shim must log no lookup of the server, no connection to its IPs and
#     no port-53 traffic. Only counts are printed, never the raw log.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
bin=$(realpath "${1:-$here/../../target/debug/zcrowd}")
hosts="zec.rocks na.zec.rocks eu.zec.rocks"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

cc -shared -fPIC -O1 -o "$work/leakcheck.so" "$here/leakcheck.c" -ldl
getent ahosts $hosts | awk '{print $1}' | sort -u >"$work/ips"
[ -s "$work/ips" ] || { echo "cannot resolve $hosts, so there are no server IPs to check against"; exit 2; }

sync() { # sync <name> <zcrowd sync flags...>: a full (all request kinds) sync of a tiny range, under the shim
  local name=$1
  shift
  LD_PRELOAD="$work/leakcheck.so" LEAKCHECK_LOG="$work/$name.log" "$bin" sync --server https://zec.rocks:443 \
    --days 0 --out "$work/$name/flows.ndjson" "$@" >"$work/$name.out" 2>&1
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

sync tor --tor-dir "$work/tordir" || { echo "Tor sync failed:"; tail -5 "$work/tor.out"; exit 1; }
t_all=$(lookups tor) t_look=$(server_lookups tor) t_ip=$(server_ips tor) t_53=$(port53 tor)
echo "Tor sync: lookups (any name)=$t_all, lookups of the server name=$t_look, connects to server IPs=$t_ip, port-53 sends=$t_53, distinct connect destinations (Tor relays)=$(dests tor)"
if [ "$t_look" -ne 0 ] || [ "$t_ip" -ne 0 ] || [ "$t_53" -ne 0 ]; then echo "LEAKCHECK FAILED"; exit 1; fi
echo "LEAKCHECK OK"
