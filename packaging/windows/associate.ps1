# Registers Savage as the current-user handler for .svg / .svgz.
# Usage: .\associate.ps1 -ExePath C:\path\to\savage.exe

param(
    [Parameter(Mandatory = $true)]
    [string]$ExePath
)

$ExePath = (Resolve-Path -LiteralPath $ExePath).Path
$progId = "Savage.SVG"
$command = '"{0}" "%1"' -f $ExePath

function Set-FileType($extension) {
    New-Item -Path "HKCU:\Software\Classes\$extension" -Force | Out-Null
    Set-ItemProperty -Path "HKCU:\Software\Classes\$extension" -Name "(default)" -Value $progId
}

New-Item -Path "HKCU:\Software\Classes\$progId\shell\open\command" -Force | Out-Null
Set-ItemProperty -Path "HKCU:\Software\Classes\$progId" -Name "(default)" -Value "SVG Document"
Set-ItemProperty -Path "HKCU:\Software\Classes\$progId\shell\open\command" -Name "(default)" -Value $command

New-Item -Path "HKCU:\Software\Classes\Applications\savage.exe\shell\open\command" -Force | Out-Null
Set-ItemProperty -Path "HKCU:\Software\Classes\Applications\savage.exe\shell\open\command" -Name "(default)" -Value $command

Set-FileType ".svg"
Set-FileType ".svgz"

Write-Host "Registered $ExePath as the current-user SVG viewer."
Write-Host "Windows Settings → Apps → Default apps may still need Savage chosen for .svg."
