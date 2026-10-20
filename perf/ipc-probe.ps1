# Servis pipe'ina baglanip istek gonderir ve yanitlari yazar (gelistirme araci).
# Komut satiri cift tirnaklari yedigi icin istekler tek tirnakla yazilir:
#   powershell -File perf\ipc-probe.ps1 "{'type':'getState'}" "{'type':'connect'}"
# Not: dosya bilerek yalnizca ASCII karakter icerir.
param([Parameter(ValueFromRemainingArguments = $true)] [string[]] $Requests)
if (-not $Requests) { $Requests = @("{'type':'hello','client':'probe','protocol':1}", "{'type':'getState'}") }
$Requests = $Requests | ForEach-Object { $_.Replace("'", '"') }

$pipe = New-Object System.IO.Pipes.NamedPipeClientStream(".", "GoodbyeDPI3", [System.IO.Pipes.PipeDirection]::InOut)
$pipe.Connect(3000)
$utf8 = New-Object System.Text.UTF8Encoding($false)
$writer = New-Object System.IO.StreamWriter($pipe, $utf8)
$writer.AutoFlush = $true
$reader = New-Object System.IO.StreamReader($pipe, $utf8)

$id = 1
foreach ($r in $Requests) {
    $writer.WriteLine('{"id":' + $id + ',"request":' + $r + '}')
    while ($true) {
        $line = $reader.ReadLine()
        if ($null -eq $line) { break }
        $line
        if ($line -match '"kind":"reply"') { break }
    }
    $id++
}
$pipe.Dispose()
