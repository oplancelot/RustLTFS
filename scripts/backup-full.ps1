# Rumba Full Backup Script (7-Zip + RustLTFS)
# Reads parameters from config.toml but performs a full backup using 7-Zip

param(
    [string]$ConfigFile = "config.toml",
    [string]$LogDir = ".\logs",
    [string]$SevenZipPath = "C:\Program Files\7-Zip\7z.exe",
    [string]$LocalTempDir = ".\Temp\LTFS_Staging"
)

$ErrorActionPreference = "Stop"

# Ensure we are running from the project root (parent of scripts dir)
Set-Location "$PSScriptRoot\.."

# Generate timestamps
$Date = Get-Date -Format "yyyyMMdd"
$SessionId = Get-Date -Format "yyyyMMdd_HHmmss"

# Create log directory
if (-not (Test-Path $LogDir)) {
    New-Item -ItemType Directory -Path $LogDir | Out-Null
}
$LogFile = Join-Path $LogDir "backup_full_$SessionId.log"
Start-Transcript -Path $LogFile -Append

Write-Host ""
Write-Host "==============================================================" -ForegroundColor Cyan
Write-Host "       Rumba Full Backup (7-Zip + RustLTFS)                   " -ForegroundColor Cyan
Write-Host "==============================================================" -ForegroundColor Cyan
Write-Host ""

# Simple TOML parser (Copied from backup-streaming.ps1)
function Get-TomlValue {
    param([string]$File, [string]$Section, [string]$Key)
    
    $content = Get-Content $File -Encoding UTF8
    $inSection = $false
    
    foreach ($line in $content) {
        $line = $line.Trim()
        
        if ($line -match "^\[$Section\]") {
            $inSection = $true
            continue
        }
        
        if ($line -match '^\[') {
            $inSection = $false
        }
        
        if ($inSection -and $line -match "^$Key\s*=\s*") {
            $parts = $line -split '=', 2
            if ($parts.Count -eq 2) {
                $val = $parts[1].Trim()
                # Handle double quotes (unescape)
                if ($val -match '^"(.*)"$') { 
                    return $matches[1] -replace '\\\\', '\'
                }
                # Handle single quotes (literal)
                if ($val -match "^'(.*)'$") { 
                    return $matches[1]
                }
                return $val
            }
        }
    }
    return $null
}

# Helper to get array from TOML (Simple implementation for excludes)
function Get-TomlArray {
    param([string]$File, [string]$Section, [string]$Key)
    
    $content = Get-Content $File -Encoding UTF8
    $inSection = $false
    $inArray = $false
    $results = @()
    
    foreach ($line in $content) {
        $line = $line.Trim()
        
        if ($line -match "^\[$Section\]") {
            $inSection = $true
            continue
        }
        
        if ($line -match '^\[') {
            $inSection = $false
        }
        
        if ($inSection) {
            if ($line -match "^$Key\s*=\s*\[") {
                $inArray = $true
                # Check for inline array: key = ["a", "b"]
                if ($line -match '\[(.*)\]') {
                    $inner = $matches[1]
                    $items = $inner -split ','
                    foreach ($item in $items) {
                        $clean = $item.Trim()
                        if ($clean -match '^"(.*)"$') { 
                            $clean = $matches[1] -replace '\\\\', '\'
                        }
                        elseif ($clean -match "^'(.*)'$") { 
                            $clean = $matches[1]
                        }
                        if ($clean) { $results += $clean }
                    }
                    return $results
                }
                continue
            }
            
            if ($inArray) {
                if ($line -match "\]") {
                    return $results
                }
                # Extract string from line (e.g., "pattern",)
                if ($line -match '"([^"]+)"') {
                    $results += $matches[1] -replace '\\\\', '\'
                }
                elseif ($line -match "'([^']+)'") {
                    $results += $matches[1]
                }
            }
        }
    }
    return $results
}

try {
    # Read configuration
    Write-Host "Reading config: $ConfigFile" -ForegroundColor Cyan
    
    $SambaSourcePath = Get-TomlValue -File $ConfigFile -Section "source" -Key "url"
    $SambaUsername = Get-TomlValue -File $ConfigFile -Section "source" -Key "username"
    $SambaPasswordEncoded = Get-TomlValue -File $ConfigFile -Section "source" -Key "password"
    
    $TapeDevice = Get-TomlValue -File $ConfigFile -Section "tape" -Key "device"
    $RustLtfsPath = Get-TomlValue -File $ConfigFile -Section "tape" -Key "rustltfs_path"
    
    # Excludes
    $Excludes = Get-TomlArray -File $ConfigFile -Section "source" -Key "excludes"
    
    # Decode password if needed
    $SambaPassword = $SambaPasswordEncoded
    if ($SambaPasswordEncoded -match "^base64:(.*)") {
        $Bytes = [System.Convert]::FromBase64String($matches[1])
        $SambaPassword = [System.Text.Encoding]::UTF8.GetString($Bytes)
    }

    # Defaults
    if (-not $TapeDevice) { $TapeDevice = "\\\\.\\TAPE1" }
    
    $ScriptRoot = $PSScriptRoot
    if (-not $RustLtfsPath) { 
        if (Test-Path "$ScriptRoot\..\rustltfs.exe") {
            $RustLtfsPath = "$ScriptRoot\..\rustltfs.exe"
        }
        else {
            $RustLtfsPath = "rustltfs.exe" 
        }
    }
    
    # Resolve paths
    $AbsRustLtfsPath = (Resolve-Path $RustLtfsPath).Path
    if (-not (Test-Path $LocalTempDir)) {
        New-Item -Path $LocalTempDir -ItemType Directory | Out-Null
    }
    
    # Define mapping drive letter
    $MappingDriveLetter = "S:"
    
    # 0. Map Network Drive
    Write-Host "--------------------------------------------------------------" -ForegroundColor Cyan
    Write-Host "Mapping Network Drive" -ForegroundColor Cyan
    Write-Host "--------------------------------------------------------------" -ForegroundColor Cyan
    
    # Cleanup existing
    if (Test-Path $MappingDriveLetter) {
        net use $MappingDriveLetter /delete /y 2>&1 | Out-Null
    }
    
    # Try mapping without credentials first (for Windows authenticated shares)
    Write-Host "   Attempting to map: $SambaSourcePath" -ForegroundColor Yellow
    $MapResult = net use $MappingDriveLetter "`"$SambaSourcePath`"" /y 2>&1
    
    # If mapping without credentials failed, try with credentials
    if ($LASTEXITCODE -ne 0 -and $SambaUsername -and $SambaPassword) {
        Write-Host "   Trying with credentials..." -ForegroundColor Yellow
        $MapResult = net use $MappingDriveLetter "`"$SambaSourcePath`"" /user:$SambaUsername $SambaPassword /y 2>&1
    }
    
    if ($LASTEXITCODE -ne 0) {
        Write-Error "Network mapping failed: $MapResult"
        throw "Failed to map network drive"
    }
    
    if (-not (Test-Path $MappingDriveLetter)) {
        throw "Failed to map network drive - drive letter not accessible"
    }
    Write-Host "   Mapped $SambaSourcePath to $MappingDriveLetter" -ForegroundColor Green
    
    # 1. Compress with 7-Zip
    Write-Host "--------------------------------------------------------------" -ForegroundColor Cyan
    Write-Host "Compressing with 7-Zip" -ForegroundColor Cyan
    Write-Host "--------------------------------------------------------------" -ForegroundColor Cyan
    
    $ArchiveName = "full_backup_$SessionId.7z"
    $LocalArchivePath = Join-Path $LocalTempDir $ArchiveName
    
    # Build 7z arguments
    # -ssw: compress shared files
    # -mx=5: normal compression
    $SevenZipArgs = @("a", "-mx=5", "-r", "-y", "-ssw", "`"$LocalArchivePath`"", "$MappingDriveLetter\*")
    
    # Add excludes
    foreach ($ex in $Excludes) {
        $cleanEx = $ex -replace "^\*\*/", ""
        $SevenZipArgs += "-xr!`"$cleanEx`""
    }
    
    Write-Host "   Archiving to: $LocalArchivePath" -ForegroundColor Yellow
    
    $Proc = Start-Process -FilePath $SevenZipPath -ArgumentList $SevenZipArgs -Wait -NoNewWindow -PassThru
    if ($Proc.ExitCode -ne 0) {
        Write-Warning "7-Zip exited with code $($Proc.ExitCode). Check logs for details."
    }
    
    if (-not (Test-Path $LocalArchivePath)) {
        throw "7-Zip failed to create archive"
    }
    
    $Size = (Get-Item $LocalArchivePath).Length / 1GB
    Write-Host "   Compression complete. Size: $([Math]::Round($Size, 2)) GB" -ForegroundColor Green
    
    # 2. Write to Tape
    Write-Host "--------------------------------------------------------------" -ForegroundColor Cyan
    Write-Host "Writing to Tape" -ForegroundColor Cyan
    Write-Host "--------------------------------------------------------------" -ForegroundColor Cyan
    
    $TapeDest = "/full_$Date/$ArchiveName"
    Write-Host "   Destination: $TapeDest" -ForegroundColor White
    
    & $AbsRustLtfsPath write --tape $TapeDevice --output $TapeDest --verify --progress $LocalArchivePath
    
    if ($LASTEXITCODE -ne 0) {
        throw "Failed to write to tape"
    }
    
    Write-Host "   Tape write complete" -ForegroundColor Green
    
    # 3. Cleanup
    Remove-Item $LocalArchivePath -Force
    Write-Host "   Cleaned up local archive" -ForegroundColor Green
}
catch {
    Write-Error "Backup Failed: $_"
    exit 1
}
finally {
    # Cleanup drive map
    if (Test-Path $MappingDriveLetter) {
        net use $MappingDriveLetter /delete /y | Out-Null
    }
    Stop-Transcript
}
