# PowerShell installer script for rag-cli
# This script detects the OS/architecture and installs or updates the rag-cli binary

param(
    [string]$Version = "latest",
    [string]$InstallDir = "$env:LOCALAPPDATA\rag-cli"
)

$ErrorActionPreference = "Stop"

# Configuration
$Repo = "MattiasHognas/tails"
$BinaryName = "rag-cli.exe"

# Helper functions
function Write-Info {
    param([string]$Message)
    Write-Host "[INFO] $Message" -ForegroundColor Green
}

function Write-Warn {
    param([string]$Message)
    Write-Host "[WARN] $Message" -ForegroundColor Yellow
}

function Write-Error-Custom {
    param([string]$Message)
    Write-Host "[ERROR] $Message" -ForegroundColor Red
}

# Detect platform
function Get-Platform {
    $arch = [System.Environment]::Is64BitOperatingSystem
    
    if (-not $arch) {
        Write-Error-Custom "32-bit Windows is not supported"
        exit 1
    }
    
    $platform = "windows-x86_64"
    Write-Info "Detected platform: $platform"
    return $platform
}

# Get download URL
function Get-DownloadUrl {
    param(
        [string]$Version,
        [string]$AssetName
    )
    
    if ($Version -eq "latest") {
        Write-Info "Fetching latest release information..."
        $releaseUrl = "https://api.github.com/repos/$Repo/releases/latest"
    } else {
        Write-Info "Fetching release information for version $Version..."
        $releaseUrl = "https://api.github.com/repos/$Repo/releases/tags/$Version"
    }
    
    try {
        $release = Invoke-RestMethod -Uri $releaseUrl -ErrorAction Stop
    } catch {
        Write-Error-Custom "Failed to fetch release information: $_"
        exit 1
    }
    
    # Find the asset for our platform
    $asset = $release.assets | Where-Object { $_.name -ieq $AssetName }
    
    if (-not $asset) {
        Write-Error-Custom "Could not find asset $AssetName in release"
        Write-Info "Available assets:"
        $release.assets | ForEach-Object { Write-Host "  - $($_.name)" }
        exit 1
    }
    
    Write-Info "Download URL: $($asset.browser_download_url)"
    return $asset.browser_download_url
}

# Check existing installation
function Test-ExistingInstallation {
    param([string]$InstallPath)
    
    if (Test-Path $InstallPath) {
        Write-Info "Existing installation found at $InstallPath"
        
        # Try to get version
        try {
            $version = & $InstallPath --version 2>$null
            Write-Info "Installed version: $version"
        } catch {
            Write-Info "Could not determine installed version"
        }
        
        return $true
    } else {
        Write-Info "No existing installation found"
        return $false
    }
}

# Download and install binary
function Install-Binary {
    param(
        [string]$DownloadUrl,
        [string]$InstallDir,
        [string]$BinaryName
    )
    
    $installPath = Join-Path $InstallDir $BinaryName
    $tempFile = Join-Path $env:TEMP "$BinaryName.tmp"
    
    Write-Info "Downloading $BinaryName..."
    try {
        Invoke-WebRequest -Uri $DownloadUrl -OutFile $tempFile -ErrorAction Stop
    } catch {
        Write-Error-Custom @"
Failed to download binary from: $DownloadUrl
Error details: $_

Common solutions:
  - Check your network connection.
  - Verify that the release exists and the URL is correct.
  - If using a proxy, ensure it is configured correctly.
"@
        exit 1
    }
    
    # Create install directory if it doesn't exist
    if (-not (Test-Path $InstallDir)) {
        Write-Info "Creating installation directory: $InstallDir"
        New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    }
    
    # Backup existing installation if it exists
    if (Test-Path $installPath) {
        Write-Info "Backing up existing installation..."
        $backupPath = "$installPath.backup"
        Move-Item -Path $installPath -Destination $backupPath -Force
    }
    
    # Move binary to installation directory
    Write-Info "Installing to $installPath..."
    try {
        Move-Item -Path $tempFile -Destination $installPath -Force
    } catch {
        Write-Error-Custom "Failed to install binary: $_"
        
        # Restore backup if it exists
        $backupPath = "$installPath.backup"
        if (Test-Path $backupPath) {
            Write-Warn "Restoring backup..."
            Move-Item -Path $backupPath -Destination $installPath -Force
        }
        exit 1
    }
    
    # Verify installation
    if (Test-Path $installPath) {
        try {
            $newVersion = & $installPath --version 2>$null
            Write-Info "Installation successful! Version: $newVersion"
        } catch {
            Write-Info "Installation successful!"
        }
        
        # Remove backup if installation was successful
        $backupPath = "$installPath.backup"
        if (Test-Path $backupPath) {
            Remove-Item $backupPath -Force
        }
    } else {
        Write-Error-Custom "Installation verification failed"
        exit 1
    }
    
    return $installPath
}

# Check and update PATH
function Update-Path {
    param([string]$InstallDir)
    
    $currentPath = [Environment]::GetEnvironmentVariable("Path", "User")
    
    if ($currentPath -notlike "*$InstallDir*") {
        Write-Warn "Installation directory is not in your PATH"
        Write-Info "Adding $InstallDir to user PATH..."
        
        try {
            $newPath = "$currentPath;$InstallDir"
            [Environment]::SetEnvironmentVariable("Path", $newPath, "User")
            Write-Info "PATH updated successfully"
            Write-Warn "Please restart your terminal for PATH changes to take effect"
        } catch {
            Write-Warn "Failed to update PATH automatically"
            Write-Info "Please add the following directory to your PATH manually:"
            Write-Host "    $InstallDir" -ForegroundColor Cyan
        }
    } else {
        Write-Info "Installation directory is already in PATH"
    }
}

# Main installation flow
function Main {
    Write-Info "Starting rag-cli installation..."
    
    $platform = Get-Platform
    $assetName = "rag-cli-$platform.exe"
    $installPath = Join-Path $InstallDir $BinaryName
    
    if (Test-ExistingInstallation -InstallPath $installPath) {
        Write-Info "Updating existing installation..."
    } else {
        Write-Info "Performing fresh installation..."
    }
    
    $downloadUrl = Get-DownloadUrl -Version $Version -AssetName $assetName
    $installedPath = Install-Binary -DownloadUrl $downloadUrl -InstallDir $InstallDir -BinaryName $BinaryName
    Update-Path -InstallDir $InstallDir
    
    Write-Info "Installation complete!"
    Write-Info "Run 'rag-cli --help' to get started"
    
    # Show current environment note
    if ($env:Path -notlike "*$InstallDir*") {
        Write-Warn "For this terminal session, use the full path:"
        Write-Host "    $installedPath --help" -ForegroundColor Cyan
    }
}

# Run main function
Main
