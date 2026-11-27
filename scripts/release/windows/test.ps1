param([Parameter(Mandatory=$true)][string]$Installer)
$ErrorActionPreference = 'Stop'
$setup = Start-Process -FilePath (Resolve-Path $Installer) -ArgumentList '/S' -Wait -PassThru
if ($setup.ExitCode -ne 0) { throw "Installer exited with $($setup.ExitCode)" }
$dir = Join-Path $env:LOCALAPPDATA 'Programs\Pawket'
$link = Join-Path ([Environment]::GetFolderPath('Programs')) 'Pawket.lnk'
$shell = New-Object -ComObject WScript.Shell
if ($shell.CreateShortcut($link).TargetPath -ne "$dir\Start Pawket.cmd") { throw 'Missing Pawket Start menu shortcut' }
$oldPath = $env:PATH
try {
    $env:PATH = "$env:SystemRoot\System32;$env:SystemRoot"
    foreach ($tool in 'deno','yt-dlp','ffmpeg') {
        $flag = if ($tool -eq 'ffmpeg') { '-version' } else { '--version' }
        & "$dir\media-tools\$tool.exe" $flag | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "$tool failed its version check" }
    }
    Start-Process -FilePath $link
    $base = 'http://127.0.0.1:3987'
    for ($i=0; $i -lt 60; $i++) {
        try { if ((Invoke-RestMethod "$base/health").status -eq 'ok') { break } } catch { }
        Start-Sleep 1
    }
    $page = (Invoke-WebRequest "$base/").Content
    $token = [regex]::Match($page, 'name="pawket-token" content="([^"]+)"').Groups[1].Value
    if (!$token) { throw 'Start menu launch did not open the Pawket service' }
    $headers = @{'X-Pawket-Token'=$token}
    $url = 'https://www.w3schools.com/html/mov_bbb.mp4'
    $info = Invoke-RestMethod "$base/v1/inspect" -Method Post -Headers $headers -ContentType 'application/json' -Body (@{url=$url}|ConvertTo-Json)
    foreach ($extension in 'mp3','mp4') {
        $format = $info.formats | Where-Object extension -eq $extension | Select-Object -First 1
        if (!$format) { throw "Missing $extension offer" }
        $result = Invoke-RestMethod "$base/v1/download" -Method Post -Headers $headers -ContentType 'application/json' -Body (@{url=$url;format_id=$format.id}|ConvertTo-Json)
        $file = Join-Path $env:TEMP "pawket-smoke.$extension"
        Invoke-WebRequest ($base+$result.url) -OutFile $file
        & "$dir\media-tools\ffmpeg.exe" -v error -i $file -f null NUL
        if ($LASTEXITCODE -ne 0) { throw "Could not decode $extension download" }
        Write-Host "Verified $extension download"
    }
} finally {
    $env:PATH = $oldPath
    Get-Process pawket -ErrorAction SilentlyContinue | Where-Object Path -eq "$dir\pawket.exe" | Stop-Process
    Start-Process "$dir\Uninstall.exe" -ArgumentList '/S' -Wait
}
