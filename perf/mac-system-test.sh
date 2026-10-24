#!/usr/bin/env bash
# Mac sistem entegrasyonu (GitHub'in Mac makinelerinde calisir):
#  1. Proxy'nin PAC dosyasi Apple'in ag katmaninda (Safari ile ayni yol) dogru okunuyor mu?
#  2. Sistem proxy'si networksetup ile yonetici sifresi istemeden degisiyor mu?
#  3. Sistem ayari PAC'i gosterince URLSession trafigi proxy'ye gidiyor mu?
#  4. Ayar geri aliniyor mu? Ekran goruntusu alinabiliyor mu?
#
#   bash perf/mac-system-test.sh [dpi-proxy yolu] [cikti klasoru]
set -u
exe="${1:-target/release/dpi-proxy}"
out="${2:-perf/out/mac-system}"
mkdir -p "$out"
port=47813
pac="http://127.0.0.1:$port/proxy.pac"
fail=0
note() { echo "$*" | tee -a "$out/summary.txt"; }
bad() {
    note "HATA: $*"
    fail=1
}

"$exe" --port "$port" --pac smart --verbose >"$out/proxy.out" 2>"$out/proxy.log" &
pid=$!
svc=""
cleanup() {
    if [ -n "$svc" ]; then
        networksetup -setautoproxystate "$svc" off >/dev/null 2>&1 || sudo -n networksetup -setautoproxystate "$svc" off
    fi
    kill "$pid" 2>/dev/null
}
trap cleanup EXIT
for _ in $(seq 1 50); do
    nc -z 127.0.0.1 "$port" 2>/dev/null && break
    sleep 0.1
done

# ------------------------------------------------------------------ 1. PAC
curl -s "$pac" -o "$out/proxy.pac" || bad "PAC indirilemedi"
grep -q FindProxyForURL "$out/proxy.pac" || bad "PAC bos"

cat >"$out/pac-eval.swift" <<'SWIFT'
import Foundation
// PAC'i CFNetwork'un JavaScriptCore'u ile degerlendirir: Safari ve URLSession ayni yolu kullanir.
let script = try! String(contentsOfFile: CommandLine.arguments[1], encoding: .utf8)
for u in CommandLine.arguments.dropFirst(2) {
    var err: Unmanaged<CFError>?
    let r = CFNetworkCopyProxiesForAutoConfigurationScript(script as CFString, URL(string: u)! as CFURL, &err)
    let list = (r?.takeRetainedValue() as? [[String: Any]]) ?? []
    let kinds = list.map { d -> String in
        let t = d[kCFProxyTypeKey as String] as? String ?? "?"
        if t == (kCFProxyTypeNone as String) { return "DIRECT" }
        return "PROXY \(d[kCFProxyHostNameKey as String] ?? "?"):\(d[kCFProxyPortNumberKey as String] ?? "?")"
    }
    print("\(u) => \(kinds.joined(separator: "; "))\(err != nil ? " (hata)" : "")")
}
SWIFT
swift "$out/pac-eval.swift" "$out/proxy.pac" \
    https://discord.com/ https://cdn.discordapp.com/x https://www.roblox.com/ https://example.com/ \
    http://192.168.1.1/ http://localhost/ >"$out/pac-eval.txt" 2>&1
cat "$out/pac-eval.txt" | tee -a "$out/summary.txt"
grep -q "^https://discord.com/ => PROXY 127.0.0.1:$port; DIRECT" "$out/pac-eval.txt" || bad "PAC: discord.com proxy'ye gitmiyor"
grep -q "^https://cdn.discordapp.com/x => PROXY 127.0.0.1:$port" "$out/pac-eval.txt" || bad "PAC: alt alan adi eslesmedi"
grep -q "^https://example.com/ => DIRECT" "$out/pac-eval.txt" || bad "PAC: listede olmayan site proxy'ye gidiyor"
grep -q "^http://192.168.1.1/ => DIRECT" "$out/pac-eval.txt" || bad "PAC: yerel ag proxy'ye gidiyor"

# ------------------------------------------------------------- 2. networksetup
svc=$(networksetup -listallnetworkservices | sed 1d | grep -v '^\*' | head -1)
note "ag hizmeti: $svc"
networksetup -getautoproxyurl "$svc" >"$out/before.txt" 2>&1
set_out=$(networksetup -setautoproxyurl "$svc" "$pac" 2>&1)
if networksetup -getautoproxyurl "$svc" | grep -q "URL: $pac"; then
    note "networksetup sudo'suz: CALISTI ($set_out)"
else
    note "networksetup sudo'suz: CALISMADI ($set_out)"
    sudo -n networksetup -setautoproxyurl "$svc" "$pac" || bad "sudo ile de ayarlanamadi"
fi
networksetup -getautoproxyurl "$svc" | tee -a "$out/summary.txt"
scutil --proxy >"$out/scutil.txt"
grep -q "ProxyAutoConfigEnable : 1" "$out/scutil.txt" || bad "sistem PAC'i etkin degil (scutil)"

# ----------------------------------------------------------- 3. URLSession
cat >"$out/fetch.swift" <<'SWIFT'
import Foundation
// Varsayilan oturum sistem proxy ayarini (PAC dahil) kullanir.
let sem = DispatchSemaphore(value: 0)
var line = "zaman asimi"
URLSession(configuration: .ephemeral).dataTask(with: URL(string: CommandLine.arguments[1])!) { _, resp, err in
    line = "\((resp as? HTTPURLResponse)?.statusCode ?? -1) \(err?.localizedDescription ?? "")"
    sem.signal()
}.resume()
_ = sem.wait(timeout: .now() + 25)
print(line)
SWIFT
sleep 1
for u in https://discord.com/ https://example.com/; do
    note "URLSession $u: $(swift "$out/fetch.swift" "$u" 2>&1 | tail -1)"
done
grep -q "discord.com:443" "$out/proxy.log" || bad "URLSession discord.com icin proxy'yi kullanmadi"
if grep -q "example.com:443" "$out/proxy.log"; then bad "example.com da proxy'den gecti (akilli liste disi)"; fi

# -------------------------------------------------------------- 4. geri alma
off_out=$(networksetup -setautoproxystate "$svc" off 2>&1)
if networksetup -getautoproxyurl "$svc" | grep -q "Enabled: No"; then
    note "geri alma sudo'suz: CALISTI ($off_out)"
else
    note "geri alma sudo'suz: CALISMADI ($off_out)"
    sudo -n networksetup -setautoproxystate "$svc" off || bad "geri alinamadi"
fi
svc=""

if screencapture -x "$out/screen.png" 2>"$out/screencapture.err"; then
    note "ekran goruntusu: $(wc -c <"$out/screen.png") bayt"
else
    note "ekran goruntusu alinamadi: $(cat "$out/screencapture.err")"
fi

note "proxy kaydi:"
cat "$out/proxy.log" | tee -a "$out/summary.txt"
exit "$fail"
