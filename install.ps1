$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$Repository = if ($env:TERMI_REPOSITORY) { $env:TERMI_REPOSITORY } else { "tuna4ll/termi" }
$Version = $env:TERMI_VERSION
$InstallDir = if ($env:TERMI_INSTALL_DIR) {
    $env:TERMI_INSTALL_DIR
} else {
    Join-Path ([Environment]::GetFolderPath("LocalApplicationData")) "Programs\termi\bin"
}

if (-not [Environment]::Is64BitOperatingSystem) {
    throw "termi: 32-bit Windows is not supported"
}

if (-not $Version) {
    $Release = Invoke-RestMethod "https://api.github.com/repos/$Repository/releases/latest"
    $Version = $Release.tag_name
}

$Target = "x86_64-pc-windows-msvc"
$Archive = "termi-$Version-$Target.zip"
$DownloadRoot = if ($env:TERMI_DOWNLOAD_ROOT) {
    $env:TERMI_DOWNLOAD_ROOT
} else {
    "https://github.com/$Repository/releases/download/$Version"
}
$TempDir = Join-Path ([IO.Path]::GetTempPath()) ("termi-" + [guid]::NewGuid())

New-Item -ItemType Directory -Path $TempDir | Out-Null
try {
    $ArchivePath = Join-Path $TempDir $Archive
    $ChecksumPath = Join-Path $TempDir "SHA256SUMS"
    Invoke-WebRequest "$DownloadRoot/$Archive" -OutFile $ArchivePath -UseBasicParsing
    Invoke-WebRequest "$DownloadRoot/SHA256SUMS" -OutFile $ChecksumPath -UseBasicParsing

    $Pattern = "^([a-fA-F0-9]{64})\s+\*?" + [regex]::Escape($Archive) + "$"
    $ChecksumLine = Get-Content $ChecksumPath | Where-Object { $_ -match $Pattern } | Select-Object -First 1
    if (-not $ChecksumLine) {
        throw "termi: checksum for $Archive is missing"
    }
    $Expected = ([regex]::Match($ChecksumLine, $Pattern).Groups[1].Value).ToLowerInvariant()
    $Actual = (Get-FileHash $ArchivePath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($Actual -ne $Expected) {
        throw "termi: checksum verification failed"
    }

    Expand-Archive $ArchivePath -DestinationPath $TempDir -Force
    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    $Binary = Join-Path $TempDir "termi-$Version-$Target\termi.exe"
    Copy-Item $Binary (Join-Path $InstallDir "termi.exe") -Force

    $UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
    $PathEntries = @($UserPath -split ";" | Where-Object { $_ })
    if ($PathEntries -notcontains $InstallDir) {
        $NewPath = if ($UserPath) { "$UserPath;$InstallDir" } else { $InstallDir }
        [Environment]::SetEnvironmentVariable("Path", $NewPath, "User")
    }
    if (@($env:Path -split ";") -notcontains $InstallDir) {
        $env:Path = "$env:Path;$InstallDir"
    }

    Write-Host "termi $Version installed to $InstallDir\termi.exe"
    Write-Host "Open a new terminal, then run: termi"
} finally {
    Remove-Item $TempDir -Recurse -Force -ErrorAction SilentlyContinue
}
