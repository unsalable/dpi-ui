#!/usr/bin/env bash
# Mac motorunun gercek bir macOS'ta calistigini dogrular (GitHub'in Mac makinelerinde calisir):
# her yontemle dpi-proxy uzerinden birkac siteye curl atar. Bu makinelerde DPI yok; olculen,
# TTL hilesinin (ters sira) ve TLS kaydi bolmenin baglantiyi bozmamasi ve ne kadar gecikme
# ekledigi. Turkiye'deki engeli asip asmadigi perf/proxy-live-test.ps1 ile olculur.
#
#   bash perf/proxy-smoke.sh [dpi-proxy yolu] [cikti klasoru]
set -u
exe="${1:-target/release/dpi-proxy}"
out="${2:-perf/out/proxy-smoke}"
mkdir -p "$out"
methods="none disorder tlsrec tlsrecdisorder disordersni splitsni split"
targets="https://example.com/ https://www.cloudflare.com/ https://www.google.com/ https://github.com/ http://example.com/"
port=47830
fail=0

check() { # yontem, etiket, curl sonucu
    printf '%-16s %-34s %s\n' "$1" "$2" "$3" | tee -a "$out/summary.txt"
    case "${3%% *}" in 000 | "") fail=1 ;; esac
}

for m in $methods; do
    port=$((port + 1))
    "$exe" --port "$port" --method "$m" --verbose >"$out/$m.out" 2>"$out/$m.log" &
    pid=$!
    for _ in $(seq 1 50); do
        nc -z 127.0.0.1 "$port" 2>/dev/null && break
        sleep 0.1
    done
    for t in $targets; do
        check "$m" "$t" "$(curl -s -o /dev/null -x "http://127.0.0.1:$port" --connect-timeout 5 --max-time 15 -w '%{http_code} %{time_total}' "$t")"
    done
    check "$m" "socks5 https://example.com/" "$(curl -s -o /dev/null --socks5-hostname "127.0.0.1:$port" --max-time 15 -w '%{http_code} %{time_total}' https://example.com/)"
    kill "$pid" 2>/dev/null
    wait "$pid" 2>/dev/null
done

if [ "$fail" -ne 0 ]; then
    echo "Bazi istekler acilmadi; ayrinti: $out" >&2
fi
exit "$fail"
